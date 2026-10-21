use binrw::prelude::*;
use maybe_async::*;

use crate::pdu::DceRpcSyntaxId;

/// Transfer syntax selected by the server for one presentation context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcTransferSyntax {
    Ndr64,
    Ndr32,
}

pub trait RpcInterface<T>
where
    T: BoundRpcConnection,
{
    const SYNTAX_ID: DceRpcSyntaxId;
    /// Syntaxes this interface can marshal. The first accepted syntax is preferred.
    const TRANSFER_SYNTAXES: &'static [DceRpcSyntaxId] = &[crate::ndr64::NDR64_SYNTAX_ID];
    fn new(bound_pipe: T) -> Self;
}

pub trait RpcCall: for<'a> BinWrite<Args<'a> = ()> {
    const OPNUM: u16;
    type ResponseType: for<'b> BinRead<Args<'b> = ()>;
    fn serialize(&self) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(vec![]);
        self.write_le(&mut cursor).unwrap();
        cursor.into_inner()
    }

    fn serialize_for(&self, _syntax: RpcTransferSyntax) -> Vec<u8> {
        self.serialize()
    }

    fn deserialize(data: &[u8]) -> crate::Result<Self::ResponseType>
    where
        Self: Sized,
    {
        let mut cursor = std::io::Cursor::new(data);
        Self::ResponseType::read_le(&mut cursor)
            .map_err(crate::SmbRpcError::FailedToParseRpcResponse)
    }

    fn deserialize_for(data: &[u8], _syntax: RpcTransferSyntax) -> crate::Result<Self::ResponseType>
    where
        Self: Sized,
    {
        Self::deserialize(data)
    }
}

#[maybe_async(AFIT)]
#[allow(async_fn_in_trait)]
pub trait BoundRpcConnection {
    fn transfer_syntax(&self) -> RpcTransferSyntax {
        RpcTransferSyntax::Ndr64
    }

    async fn send_receive<S>(&mut self, stub_input: S) -> crate::Result<S::ResponseType>
    where
        S: RpcCall,
    {
        let syntax = self.transfer_syntax();
        let serialized_input = stub_input.serialize_for(syntax);
        let response = self.send_receive_raw(S::OPNUM, &serialized_input).await?;
        S::deserialize_for(&response, syntax)
    }

    async fn send_receive_raw(&mut self, opnum: u16, stub_input: &[u8]) -> crate::Result<Vec<u8>>;
}
