use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

use super::ResourceHandle;
use crate::msg_handler::{OutgoingMessage, ReceiveOptions};
use maybe_async::*;
use smb_msg::{IoctlBuffer, PipeTransceiveRequest, ReadRequest, WriteRequest};
use smb_rpc::{
    interface::{RpcTransferSyntax, *},
    ndr32::NDR32_SYNTAX_ID,
    ndr64::NDR64_SYNTAX_ID,
    pdu::*,
    SmbRpcError,
};
pub struct Pipe {
    handle: ResourceHandle,
}

#[maybe_async(AFIT)]
impl Pipe {
    pub fn new(handle: ResourceHandle) -> Self {
        Pipe { handle }
    }

    pub async fn bind<I>(self) -> crate::Result<I>
    where
        I: RpcInterface<PipeRpcConnection>,
    {
        PipeRpcConnection::bind::<I>(self).await
    }
}

pub struct PipeRpcConnection {
    pipe: Pipe,
    next_call_id: u32,
    /// Selected, accepted, context ID from binding.
    context_id: u16,
    transfer_syntax: RpcTransferSyntax,

    server_max_xmit_frag: u16,
    _server_max_recv_frag: u16,
}

#[maybe_async(AFIT)]
impl PipeRpcConnection {
    pub async fn bind<I>(mut pipe: Pipe) -> crate::Result<I>
    where
        I: RpcInterface<PipeRpcConnection>,
    {
        let transfer_syntaxes = I::TRANSFER_SYNTAXES;
        if transfer_syntaxes.is_empty() {
            return Err(crate::Error::InvalidMessage(
                "RPC interface offered no transfer syntaxes".to_string(),
            ));
        }
        let context_elements = Self::make_bind_contexts(I::SYNTAX_ID, transfer_syntaxes);

        const START_CALL_ID: u32 = 2;
        const DEFAULT_FRAG_LIMIT: u16 = 4280;
        const NO_ASSOC_GROUP_ID: u32 = 0;
        let bind_ack = Self::rpc_rw(
            &mut pipe,
            START_CALL_ID,
            DcRpcCoPktBind {
                max_xmit_frag: DEFAULT_FRAG_LIMIT,
                max_recv_frag: DEFAULT_FRAG_LIMIT,
                assoc_group_id: NO_ASSOC_GROUP_ID,
                context_elements,
            }
            .into(),
        )
        .await?;

        let bind_ack = match bind_ack.content() {
            DcRpcCoPktResponseContent::BindAck(bind_ack) => {
                log::debug!("Bounded to pipe with port spec {}", bind_ack.port_spec);
                bind_ack
            }
            _ => {
                return Err(crate::Error::InvalidMessage(format!(
                    "Expected BindAck, got: {bind_ack:?}",
                )));
            }
        };

        let (context_id, transfer_syntax) = Self::check_bind_results(bind_ack, transfer_syntaxes)?;

        Ok(I::new(PipeRpcConnection {
            pipe,
            next_call_id: START_CALL_ID + 1,
            context_id,
            transfer_syntax,
            server_max_xmit_frag: bind_ack.max_xmit_frag,
            _server_max_recv_frag: bind_ack.max_recv_frag,
        }))
    }

    fn make_bind_contexts(
        syntax_id: DceRpcSyntaxId,
        transfer_syntaxes: &[DceRpcSyntaxId],
    ) -> Vec<DcRpcCoPktBindContextElement> {
        let mut contexts = transfer_syntaxes
            .iter()
            .enumerate()
            .map(|(index, syntax)| DcRpcCoPktBindContextElement {
                context_id: index as u16,
                abstract_syntax: syntax_id.clone(),
                transfer_syntaxes: vec![syntax.clone()],
            })
            .collect::<Vec<_>>();
        // Bind-time feature negotiation remains a separate context for compatibility with the existing client.
        contexts.push(DcRpcCoPktBindContextElement {
            context_id: contexts.len() as u16,
            abstract_syntax: syntax_id,
            transfer_syntaxes: vec![BIND_TIME_NEGOTIATION],
        });
        contexts
    }

    fn check_bind_results(
        bind_ack: &DcRpcCoPktBindAck,
        transfer_syntaxes: &[DceRpcSyntaxId],
    ) -> crate::Result<(u16, RpcTransferSyntax)> {
        if bind_ack.results.len() != transfer_syntaxes.len() + 1 {
            return Err(crate::Error::InvalidMessage(format!(
                "BindAck results length {} does not match the {} offered presentation contexts",
                bind_ack.results.len(),
                transfer_syntaxes.len() + 1
            )));
        }
        let mut selected = None;
        for (context_id, (result, syntax)) in
            bind_ack.results.iter().zip(transfer_syntaxes).enumerate()
        {
            if result.result != DceRpcCoPktBindAckDefResult::Acceptance {
                log::debug!("RPC transfer syntax {syntax} rejected: {result:?}");
                continue;
            }
            if result.syntax != *syntax {
                return Err(crate::Error::InvalidMessage(format!(
                    "BindAck accepted {} for context offered as {syntax}",
                    result.syntax
                )));
            }
            let transfer_syntax = if syntax.uuid == NDR32_SYNTAX_ID.uuid {
                RpcTransferSyntax::Ndr32
            } else if syntax.uuid == NDR64_SYNTAX_ID.uuid {
                RpcTransferSyntax::Ndr64
            } else {
                return Err(crate::Error::InvalidMessage(format!(
                    "Unsupported accepted transfer syntax: {syntax}"
                )));
            };
            // Prefer the first accepted syntax, which is the caller's declared priority order.
            if selected.is_none() {
                selected = Some((context_id as u16, transfer_syntax));
            }
        }
        let bind_time_result = bind_ack.results.last().unwrap();
        log::debug!(
            "Bind time feature negotiation result: {:?}",
            bind_time_result.result
        );
        let selected = selected.ok_or_else(|| {
            crate::Error::InvalidMessage(
                "BindAck rejected all offered RPC transfer syntaxes".to_string(),
            )
        })?;
        log::debug!("Selected context ID {} with {:?}", selected.0, selected.1);
        Ok(selected)
    }

    pub const PACKED_DREP: u32 = 0x10;
    /// Performs a read+write operation on the pipe, sending a request and receiving it's response.
    #[maybe_async]
    async fn rpc_rw(
        pipe: &mut Pipe,
        call_id: u32,
        to_send: DcRpcCoPktRequestContent,
    ) -> crate::Result<DceRpcCoResponsePkt> {
        const READ_WRITE_PIPE_OFFSET: u64 = 0;
        let file_id = pipe.handle.file_id()?;
        let dcerpc_request_buffer: Vec<u8> = DceRpcCoRequestPkt::new(
            to_send,
            call_id,
            DceRpcCoPktFlags::new()
                .with_first_frag(true)
                .with_last_frag(true),
            Self::PACKED_DREP,
        )
        .try_into()?;
        let exp_write_size = dcerpc_request_buffer.len() as u32;
        let write_result = pipe
            .sendo_recvo(
                OutgoingMessage::new(
                    WriteRequest::new(
                        READ_WRITE_PIPE_OFFSET,
                        file_id,
                        Default::default(),
                        dcerpc_request_buffer.len() as u32,
                    )
                    .into(),
                )
                .with_additional_data(Arc::from(dcerpc_request_buffer)),
                ReceiveOptions::new().with_allow_async(true),
            )
            .await?;
        if write_result.message.content.to_write()?.count != exp_write_size {
            return Err(crate::Error::InvalidMessage(
                "Failed to write the full request to the pipe".to_string(),
            ));
        }

        let read_result = pipe
            .send_recvo(
                ReadRequest {
                    flags: Default::default(),
                    length: 1024,
                    offset: READ_WRITE_PIPE_OFFSET,
                    file_id,
                    minimum_count: DceRpcCoRequestPkt::COMMON_SIZE_BYTES as u32,
                }
                .into(),
                ReceiveOptions::new().with_allow_async(true),
            )
            .await?;
        let content = read_result.message.content.to_read()?;
        let response = DceRpcCoResponsePkt::try_from(content.buffer.as_ref())?;

        if response.packed_drep() != Self::PACKED_DREP {
            return Err(crate::Error::InvalidMessage(format!(
                "Currently Unsupported packed DREP: {}",
                response.packed_drep()
            )));
        }

        if !response.pfc_flags().first_frag() || !response.pfc_flags().last_frag() {
            return Err(crate::Error::InvalidMessage(
                "Expected first and last fragment flags to be set".to_string(),
            ));
        }

        Ok(response)
    }

    pub fn pipe(&self) -> &Pipe {
        &self.pipe
    }
}

impl BoundRpcConnection for PipeRpcConnection {
    fn transfer_syntax(&self) -> RpcTransferSyntax {
        self.transfer_syntax
    }

    #[maybe_async]
    async fn send_receive_raw(
        &mut self,
        opnum: u16,
        stub_input: &[u8],
    ) -> Result<Vec<u8>, SmbRpcError> {
        let alloc_hint = u32::try_from(stub_input.len()).map_err(|_| {
            SmbRpcError::SendReceiveError(
                "RPC request stub exceeds the allocation hint limit".to_string(),
            )
        })?;
        let req = DcRpcCoPktRequest {
            alloc_hint,
            context_id: self.context_id,
            opnum,
            stub_data: stub_input.to_vec(),
        }
        .into();
        let req = DceRpcCoRequestPkt::new(
            req,
            self.next_call_id,
            DceRpcCoPktFlags::new()
                .with_first_frag(true)
                .with_last_frag(true),
            Self::PACKED_DREP, // Packed DREP
        );
        self.next_call_id += 1;

        let req_data: Vec<u8> = req.try_into().map_err(|e| {
            SmbRpcError::SendReceiveError(format!("Failed to serialize RPC request: {e}"))
        })?;

        let res = self
            .pipe
            .handle
            .fsctl_with_options(
                PipeTransceiveRequest::from(IoctlBuffer::from(req_data)),
                self.server_max_xmit_frag as u32,
            )
            .await
            .map_err(|e| {
                SmbRpcError::SendReceiveError(format!("Failed to send RPC request via FSCTL: {e}"))
            })?;

        let rpc_reply = DceRpcCoResponsePkt::try_from(res.as_ref())
            .map_err(SmbRpcError::FailedToParseRpcResponse)?;

        if rpc_reply.packed_drep() != Self::PACKED_DREP {
            return Err(SmbRpcError::SendReceiveError(format!(
                "Currently Unsupported packed DREP: {}",
                rpc_reply.packed_drep()
            )));
        }

        if !rpc_reply.pfc_flags().first_frag() || !rpc_reply.pfc_flags().last_frag() {
            return Err(SmbRpcError::SendReceiveError(
                "Expected first and last fragment flags to be set".to_string(),
            ));
        }

        let response = match rpc_reply.into_content() {
            DcRpcCoPktResponseContent::Response(dc_rpc_co_pkt_response) => dc_rpc_co_pkt_response,
            DcRpcCoPktResponseContent::Fault(fault) => {
                return Err(SmbRpcError::SendReceiveError(format!(
                    "DCE/RPC fault status {:#010X} for context {} (alloc_hint {}, reserved {:#010X})",
                    fault.status, fault.context_id, fault.alloc_hint, fault.reserved
                )));
            }
            content => {
                return Err(SmbRpcError::SendReceiveError(format!(
                    "Expected DceRpcCoPktResponseContent::Response, got: {content:?}",
                )));
            }
        };

        if response.context_id != self.context_id {
            return Err(SmbRpcError::SendReceiveError(format!(
                "Response context ID {} does not match expected {}",
                response.context_id, self.context_id
            )));
        }

        Ok(response.stub_data)
    }
}

impl Deref for Pipe {
    type Target = ResourceHandle;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl DerefMut for Pipe {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.handle
    }
}

#[cfg(test)]
mod tests {
    use super::PipeRpcConnection;
    use smb_rpc::interface::RpcTransferSyntax;
    use smb_rpc::{ndr32::NDR32_SYNTAX_ID, ndr64::NDR64_SYNTAX_ID, pdu::*};

    #[test]
    fn bind_pdu_offers_ndr64_then_ndr32_in_separate_contexts() {
        let transfer_syntaxes = [NDR64_SYNTAX_ID, NDR32_SYNTAX_ID];
        let contexts = PipeRpcConnection::make_bind_contexts(
            DceRpcSyntaxId {
                uuid: smb_dtyp::make_guid!("4b324fc8-1670-01d3-1278-5a47bf6ee188"),
                version: 3,
            },
            &transfer_syntaxes,
        );
        let packet = DceRpcCoRequestPkt::new(
            DcRpcCoPktBind {
                max_xmit_frag: 4280,
                max_recv_frag: 4280,
                assoc_group_id: 0,
                context_elements: contexts,
            }
            .into(),
            2,
            DceRpcCoPktFlags::new()
                .with_first_frag(true)
                .with_last_frag(true),
            PipeRpcConnection::PACKED_DREP,
        );
        let bytes: Vec<u8> = packet.try_into().expect("encode bind PDU");
        let decoded = DceRpcCoRequestPkt::try_from(bytes.as_slice()).expect("decode bind PDU");
        let DcRpcCoPktRequestContent::Bind(bind) = decoded.into_content() else {
            panic!("expected a bind PDU");
        };
        assert_eq!(bind.context_elements.len(), 3);
        assert_eq!(bind.context_elements[0].context_id, 0);
        assert_eq!(
            bind.context_elements[0].transfer_syntaxes,
            [NDR64_SYNTAX_ID]
        );
        assert_eq!(bind.context_elements[1].context_id, 1);
        assert_eq!(
            bind.context_elements[1].transfer_syntaxes,
            [NDR32_SYNTAX_ID]
        );
    }

    #[test]
    fn ndr32_is_selected_when_bind_ack_rejects_ndr64() {
        let transfer_syntaxes = [NDR64_SYNTAX_ID, NDR32_SYNTAX_ID];
        let bind_ack = DcRpcCoPktBindAck {
            max_xmit_frag: 4280,
            max_recv_frag: 4280,
            assoc_group_id: 0,
            port_spec: "\\PIPE\\srvsvc\0".into(),
            results: vec![
                DcRpcCoPktBindAckResult {
                    result: DceRpcCoPktBindAckDefResult::ProviderRejection,
                    reason: DcRpcCoPktBindAckReason::ProposedTransferSyntaxesNotSupported,
                    syntax: DceRpcSyntaxId::ZERO,
                },
                DcRpcCoPktBindAckResult {
                    result: DceRpcCoPktBindAckDefResult::Acceptance,
                    reason: DcRpcCoPktBindAckReason::NotSpecified,
                    syntax: NDR32_SYNTAX_ID,
                },
                DcRpcCoPktBindAckResult {
                    result: DceRpcCoPktBindAckDefResult::NegotiateAck,
                    reason: DcRpcCoPktBindAckReason::NotSpecified,
                    syntax: DceRpcSyntaxId::ZERO,
                },
            ],
        };
        assert_eq!(
            PipeRpcConnection::check_bind_results(&bind_ack, &transfer_syntaxes).unwrap(),
            (1, RpcTransferSyntax::Ndr32)
        );
    }
}
