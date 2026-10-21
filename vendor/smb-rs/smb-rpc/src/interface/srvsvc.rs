#![allow(unused_parens)]

use crate::{interface::*, pdu::DceRpcSyntaxId};
use smb_dtyp::make_guid;

use crate::interface::RpcTransferSyntax;
use crate::ndr64::*;
use binrw::prelude::*;
use maybe_async::maybe_async;
use modular_bitfield::prelude::*;
use std::collections::HashSet;

const ERROR_SUCCESS: u32 = 0;
const ERROR_MORE_DATA: u32 = 234;
const MAX_SHARE_ENUM_ENTRIES: usize = 65_536;
const MAX_SHARE_ENUM_PAGES: usize = 1_024;
const MAX_SHARE_ENUM_STRING_UNITS: usize = 4_194_304;
/// [SHARE_ENUM_STRUCT](<https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-srvs/79ee052e-e16b-4ec5-b4b7-e99777c26eca>)
#[binrw::binrw]
#[derive(Debug, PartialEq, Eq)]
struct ShareEnumStruct {
    #[bw(calc = share_info.level().into())]
    level: NdrAlign<ShareInfoLevel>,
    #[br(args(*level))]
    share_info: NdrAlign<ShareEnumUnion>,
}

#[binrw::binrw]
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[brw(repr(u32))]
pub enum ShareInfoLevel {
    Info0 = 0,
    Info1 = 1,
    Info2 = 2,
    Info501 = 501,
    Info502 = 502,
    Info503 = 503,
}

/// [`SHARE_ENUM_UNION`](<https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-srvs/7894d7e4-bb82-419c-b431-0247c8ae4dfe>)
#[binrw::binrw]
#[derive(Debug, PartialEq, Eq)]
#[br(import(level: ShareInfoLevel))]
enum ShareEnumUnion {
    #[brw(magic = 0u64)]
    #[br(pre_assert(level == ShareInfoLevel::Info0))]
    Info0(NdrPtr<ShareInfoContainer<ShareInfo0>>),
    #[brw(magic = 1u64)]
    #[br(pre_assert(level == ShareInfoLevel::Info1))]
    Info1(NdrPtr<ShareInfoContainer<ShareInfo1>>),
}

impl ShareEnumUnion {
    /// Returns the level of the share info contained in this union.
    pub fn level(&self) -> ShareInfoLevel {
        match self {
            ShareEnumUnion::Info0(_) => ShareInfoLevel::Info0,
            ShareEnumUnion::Info1(_) => ShareInfoLevel::Info1,
        }
    }
}

/// [`SHARE_INFO_1_CONTAINER`](<https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-srvs/919abd5d-87d9-4ffa-b4b1-632a66053bc6>)
#[binrw::binrw]
#[derive(Debug, PartialEq, Eq)]
struct ShareInfoContainer<T>
where
    T: ShareInfo,
{
    #[bw(calc = (buffer.as_ref().map_or(0, |x| x.len() as u32)).into())]
    entries_read: NdrAlign<u32>,
    #[br(args(None, NdrPtrReadMode::NoArraySupport, (*entries_read as u64,)))]
    buffer: NdrPtr<NdrArray<T>>,
}

trait ShareInfo:
    for<'a> BinRead<Args<'a> = (Option<&'a Self>,)>
    + for<'a> BinWrite<Args<'a> = (NdrPtrWriteStage,)>
    + Clone
    + PartialEq
    + Eq
    + 'static
{
}

/// [`SHARE_INFO_1`](<https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-srvs/fc69f110-998d-4c16-9667-514e22fdd80b>)
#[binrw::binrw]
#[derive(Debug, PartialEq, Eq, Clone)]
#[bw(import(stage: NdrPtrWriteStage))]
#[br(import(prev: Option<&Self>))]
pub struct ShareInfo1 {
    #[bw(args_raw(NdrPtrWriteArgs(stage, ())))]
    #[br(args(prev.map(|x| &x.netname), NdrPtrReadMode::WithArraySupport, ()))]
    pub netname: NdrPtr<NdrString<u16>>,
    #[bw(if(stage == NdrPtrWriteStage::ArraySupportWriteRefId))]
    #[br(args(prev.map(|x| &**x.share_type)))]
    pub share_type: NdrArrayStructureElement<ShareType>,
    #[bw(args_raw(NdrPtrWriteArgs(stage, ())))]
    #[br(args(prev.map(|x| &x.remark), NdrPtrReadMode::WithArraySupport, ()))]
    pub remark: NdrPtr<NdrString<u16>>,
}

/// [`SHARE_INFO_0`](<https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-srvs/73a25288-8086-4975-91a3-5cbee5b590cc>)
#[binrw::binrw]
#[derive(Debug, PartialEq, Eq, Clone)]
#[bw(import(stage: NdrPtrWriteStage))]
#[br(import(prev: Option<&Self>))]
pub struct ShareInfo0 {
    #[bw(args_raw(NdrPtrWriteArgs(stage, ())))]
    #[br(args(prev.map(|x| &x.netname), NdrPtrReadMode::WithArraySupport, ()))]
    netname: NdrPtr<NdrString<u16>>,
}

impl ShareInfo for ShareInfo0 {}
impl ShareInfo for ShareInfo1 {}

#[derive(BitfieldSpecifier, Debug, Clone, Copy, PartialEq, Eq)]
#[bits = 2]
pub enum ShareKind {
    Disk = 0,
    PrintQ = 1,
    Device = 2,
    IPC = 3,
}

/// Share types
///
/// [MS-SRVS](<https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-srvs/6069f8c0-c93f-43a0-a5b4-7ed447eb4b84>)
#[smb_dtyp::mbitfield]
pub struct ShareType {
    pub kind: ShareKind,
    #[skip]
    __: B23,
    pub cluster_fs: bool,
    pub cluster_sofs: bool,
    pub cluster_dfs: bool,
    #[skip]
    __: B2,
    pub temporary: bool,
    pub special: bool,
}

impl ShareType {
    /// Returns whether this is the windows IPC share (IPC$)
    pub fn is_win_ipc(&self) -> bool {
        self.kind() == ShareKind::IPC && self.special()
    }
}

// FYI: RPC top-level stub data is aligned to min(8, arg_size0, arg_size1, ...) bytes.
// DCE/RPC Chap. 12.3: RPC PDU Encodings/Alignment.

/// Input arguments for [NetrShareEnum](<https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-srvs/c4a98e7b-d416-439c-97bd-4d9f52f8ba52>)
#[binrw::binrw]
#[derive(Debug, PartialEq, Eq)]
struct NetrShareEnumIn {
    server_name: NdrAlign<NdrPtr<NdrString<u16>>, 4>,
    info_struct: NdrAlign<ShareEnumStruct, 4>,
    prefered_maximum_length: NdrAlign<u32, 4>,
    resume_handle: NdrAlign<NdrPtr<u32>, 4>,
}

/// Return value and out params of [NetrShareEnum](<https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-srvs/c4a98e7b-d416-439c-97bd-4d9f52f8ba52>)
#[binrw::binrw]
#[derive(Debug, PartialEq, Eq)]
struct NetrShareEnumOut {
    info_struct: NdrAlign<ShareEnumStruct, 4>,
    total_entries: NdrAlign<u32, 4>,
    resume_handle: NdrAlign<NdrPtr<u32>, 4>,
    status: NdrAlign<u32, 4>,
}

impl RpcCall for NetrShareEnumIn {
    const OPNUM: u16 = 0xf;

    type ResponseType = NetrShareEnumOut;

    fn serialize_for(&self, syntax: RpcTransferSyntax) -> Vec<u8> {
        match syntax {
            RpcTransferSyntax::Ndr64 => self.serialize(),
            RpcTransferSyntax::Ndr32 => serialize_netr_share_enum_ndr32(self),
        }
    }

    fn deserialize_for(
        data: &[u8],
        syntax: RpcTransferSyntax,
    ) -> crate::Result<Self::ResponseType> {
        match syntax {
            RpcTransferSyntax::Ndr64 => deserialize_netr_share_enum_ndr64(data),
            RpcTransferSyntax::Ndr32 => deserialize_netr_share_enum_ndr32(data),
        }
    }
}

fn deserialize_netr_share_enum_ndr64(data: &[u8]) -> crate::Result<NetrShareEnumOut> {
    // NetrShareEnum's NDR64 output starts with SHARE_ENUM_STRUCT. When its container
    // pointer is non-null, entries_read is the u32 at offset 24. Check it before the
    // generic NdrArray reader uses that value for Vec::with_capacity.
    let expected_entries = if data.len() >= 28 {
        let container_ref = u64::from_le_bytes(data[16..24].try_into().unwrap());
        let level = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let switch = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let supported_union = matches!((level, switch), (0, 0) | (1, 1));
        if container_ref != 0 && supported_union {
            let entries_read = u32::from_le_bytes(data[24..28].try_into().unwrap()) as usize;
            if entries_read > MAX_SHARE_ENUM_ENTRIES {
                return Err(crate::SmbRpcError::InvalidResponseData(
                    "NetrShareEnum returned an invalid share count",
                ));
            }
            Some(entries_read)
        } else {
            None
        }
    } else {
        None
    };

    let mut cursor = std::io::Cursor::new(data);
    let response = NetrShareEnumOut::read_le(&mut cursor)
        .map_err(crate::SmbRpcError::FailedToParseRpcResponse)?;
    if cursor.position() != data.len() as u64 {
        return Err(crate::SmbRpcError::InvalidResponseData(
            "NetrShareEnum returned trailing response data",
        ));
    }
    if let Some(expected_entries) = expected_entries
        && response.shares()?.len() != expected_entries
    {
        return Err(crate::SmbRpcError::InvalidResponseData(
            "NetrShareEnum share count did not match its buffer",
        ));
    }
    Ok(response)
}

fn serialize_netr_share_enum_ndr32(input: &NetrShareEnumIn) -> Vec<u8> {
    let server_name = input
        .server_name
        .as_ref()
        .expect("NetrShareEnum requires a server name")
        .data
        .value
        .iter()
        .copied()
        .take_while(|unit| *unit != 0)
        .collect::<Vec<_>>();
    let count = (server_name.len() + 1) as u32;
    let mut bytes = Vec::with_capacity(4 + 12 + count as usize * 2 + 12 + 8 + 8);
    // PIDL emits each RPC argument in order. The unique server-name pointer's string referent
    // follows its pointer ID before the next argument's fixed fields.
    bytes.extend_from_slice(&0x0002_0000u32.to_le_bytes());
    // [string] referent: conformant-varying UTF-16 array.
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    for unit in server_name.into_iter().chain(std::iter::once(0)) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    // SHARE_ENUM_STRUCT: level and the non-encapsulated union discriminator are both encoded,
    // followed by the unique container pointer. The union referent is emitted with this struct.
    for value in [1u32, 1, 0x0002_0004] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    // Deferred SHARE_INFO_1_CONTAINER, with no input buffer.
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    // Remaining input arguments: preferred maximum length and a non-null resume handle.
    // NetrShareEnum uses the handle as an in/out value; the first request starts at zero.
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    let resume_handle = input
        .resume_handle
        .as_ref()
        .map(|value| **value)
        .unwrap_or(0);
    bytes.extend_from_slice(&0x0002_0010u32.to_le_bytes());
    bytes.extend_from_slice(&resume_handle.to_le_bytes());
    bytes
}

fn deserialize_netr_share_enum_ndr32(data: &[u8]) -> crate::Result<NetrShareEnumOut> {
    let mut reader = Ndr32Reader::new(data);
    let level = reader.u32()?;
    let switch = reader.u32()?;
    let container_ref = reader.u32()?;
    if level != ShareInfoLevel::Info1 as u32 || switch != ShareInfoLevel::Info1 as u32 {
        return Err(crate::SmbRpcError::InvalidResponseData(
            "NetrShareEnum returned an unsupported share info level",
        ));
    }
    let mut shares = Vec::new();
    if container_ref != 0 {
        // The [in,out,ref] info structure is serialized as one RPC argument: its unique
        // container referent and array data precede the next output argument, total_entries.
        let entries_read = reader.u32()? as usize;
        let buffer_ref = reader.u32()?;
        if buffer_ref == 0 {
            if entries_read != 0 {
                return Err(crate::SmbRpcError::InvalidResponseData(
                    "NetrShareEnum returned entries without a share buffer",
                ));
            }
        } else {
            let max_count = reader.u32()? as usize;
            if entries_read > max_count || entries_read > MAX_SHARE_ENUM_ENTRIES {
                return Err(crate::SmbRpcError::InvalidResponseData(
                    "NetrShareEnum returned an invalid share count",
                ));
            }

            let mut inline_entries = Vec::with_capacity(entries_read);
            for _ in 0..entries_read {
                let netname_ref = reader.u32()?;
                let share_type = reader.u32()?;
                let remark_ref = reader.u32()?;
                inline_entries.push((netname_ref, share_type, remark_ref));
            }

            for (netname_ref, share_type, remark_ref) in inline_entries {
                let netname = if netname_ref == 0 {
                    None
                } else {
                    Some(reader.utf16_string()?)
                };
                let remark = if remark_ref == 0 {
                    None
                } else {
                    Some(reader.utf16_string()?)
                };
                let netname = netname
                    .ok_or(crate::SmbRpcError::InvalidResponseData(
                        "NetrShareEnum returned a null share name",
                    ))?
                    .parse::<NdrString<u16>>()
                    .map_err(|_| {
                        crate::SmbRpcError::InvalidResponseData("invalid UTF-16 share name")
                    })?;
                let remark = remark
                    .unwrap_or_default()
                    .parse::<NdrString<u16>>()
                    .map_err(|_| {
                        crate::SmbRpcError::InvalidResponseData("invalid UTF-16 share remark")
                    })?;
                shares.push(ShareInfo1 {
                    netname: NdrPtr::from(netname),
                    share_type: NdrArrayStructureElement::from(ShareType::from_bytes(
                        share_type.to_le_bytes(),
                    )),
                    remark: NdrPtr::from(remark),
                });
            }
        }
    }

    let total_entries = reader.u32()?;
    let resume_ref = reader.u32()?;
    let resume_handle = if resume_ref == 0 {
        None
    } else {
        Some(reader.u32()?)
    };
    let status = reader.u32()?;
    reader.finish()?;
    Ok(NetrShareEnumOut {
        info_struct: ShareEnumStruct {
            share_info: ShareEnumUnion::Info1(NdrPtr::from(if container_ref == 0 {
                None
            } else {
                Some(ShareInfoContainer::<ShareInfo1> {
                    buffer: NdrPtr::from(NdrArray::from(shares)),
                })
            }))
            .into(),
        }
        .into(),
        total_entries: total_entries.into(),
        resume_handle: NdrPtr::<u32>::from(resume_handle).into(),
        status: status.into(),
    })
}

struct Ndr32Reader<'a> {
    data: &'a [u8],
    offset: usize,
    string_units: usize,
}

impl<'a> Ndr32Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            string_units: 0,
        }
    }

    fn u32(&mut self) -> crate::Result<u32> {
        self.align(4)?;
        let end = self
            .offset
            .checked_add(4)
            .ok_or(crate::SmbRpcError::InvalidResponseData(
                "NDR32 offset overflow",
            ))?;
        let bytes =
            self.data
                .get(self.offset..end)
                .ok_or(crate::SmbRpcError::InvalidResponseData(
                    "truncated NDR32 response",
                ))?;
        self.offset = end;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn align(&mut self, alignment: usize) -> crate::Result<()> {
        let aligned = self
            .offset
            .checked_add(alignment - 1)
            .map(|value| value & !(alignment - 1))
            .ok_or(crate::SmbRpcError::InvalidResponseData(
                "NDR32 offset overflow",
            ))?;
        if aligned > self.data.len() {
            return Err(crate::SmbRpcError::InvalidResponseData(
                "truncated NDR32 alignment padding",
            ));
        }
        self.offset = aligned;
        Ok(())
    }

    fn utf16_string(&mut self) -> crate::Result<String> {
        self.align(4)?;
        let max_count = self.u32()? as usize;
        let offset = self.u32()? as usize;
        let actual_count = self.u32()? as usize;
        if offset != 0 || actual_count == 0 || actual_count > max_count || actual_count > 1_048_576
        {
            return Err(crate::SmbRpcError::InvalidResponseData(
                "invalid NDR32 UTF-16 string bounds",
            ));
        }
        self.string_units = self
            .string_units
            .checked_add(actual_count)
            .filter(|units| *units <= MAX_SHARE_ENUM_STRING_UNITS)
            .ok_or(crate::SmbRpcError::InvalidResponseData(
                "NetrShareEnum returned too much string data",
            ))?;
        let byte_len =
            actual_count
                .checked_mul(2)
                .ok_or(crate::SmbRpcError::InvalidResponseData(
                    "NDR32 string length overflow",
                ))?;
        let end =
            self.offset
                .checked_add(byte_len)
                .ok_or(crate::SmbRpcError::InvalidResponseData(
                    "NDR32 offset overflow",
                ))?;
        let bytes =
            self.data
                .get(self.offset..end)
                .ok_or(crate::SmbRpcError::InvalidResponseData(
                    "truncated NDR32 UTF-16 string",
                ))?;
        self.offset = end;
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        if units.last() != Some(&0) || units[..units.len() - 1].contains(&0) {
            return Err(crate::SmbRpcError::InvalidResponseData(
                "invalid NDR32 UTF-16 string terminator",
            ));
        }
        String::from_utf16(&units[..units.len() - 1])
            .map_err(|_| crate::SmbRpcError::InvalidResponseData("invalid NDR32 UTF-16 string"))
    }

    fn finish(&self) -> crate::Result<()> {
        if self.offset != self.data.len() {
            return Err(crate::SmbRpcError::InvalidResponseData(
                "NetrShareEnum returned trailing response data",
            ));
        }
        Ok(())
    }
}

pub struct SrvSvc<T>
where
    T: BoundRpcConnection,
{
    bound_pipe: T,
}

impl<T> SrvSvc<T>
where
    T: BoundRpcConnection,
{
    /// The transfer syntax accepted by the server for this SRVSVC pipe.
    pub fn transfer_syntax(&self) -> RpcTransferSyntax {
        self.bound_pipe.transfer_syntax()
    }

    #[maybe_async]
    pub async fn netr_share_enum(&mut self, server_name: &str) -> crate::Result<Vec<ShareInfo1>> {
        let mut resume_handle = 0u32;
        let mut seen_resume_handles = HashSet::new();
        let mut result = Vec::new();

        for page_number in 0..MAX_SHARE_ENUM_PAGES {
            let input_struct = NetrShareEnumIn {
                server_name: NdrPtr::from(server_name.parse::<NdrString<u16>>().map_err(|_| {
                    crate::SmbRpcError::InvalidResponseData("invalid NetrShareEnum server name")
                })?)
                .into(),
                info_struct: ShareEnumStruct {
                    share_info: ShareEnumUnion::Info1(NdrPtr::from(ShareInfoContainer::<
                        ShareInfo1,
                    > {
                        buffer: NdrPtr::from(None),
                    }))
                    .into(),
                }
                .into(),
                prefered_maximum_length: u32::MAX.into(),
                resume_handle: NdrPtr::from(resume_handle).into(),
            };
            let page = self.bound_pipe.send_receive(input_struct).await?;
            match page.append_to(&mut result, resume_handle, &mut seen_resume_handles)? {
                None => return Ok(result),
                Some(next_resume_handle) => resume_handle = next_resume_handle,
            }

            if page_number + 1 == MAX_SHARE_ENUM_PAGES {
                return Err(crate::SmbRpcError::InvalidResponseData(
                    "NetrShareEnum exceeded the page limit",
                ));
            }
        }

        unreachable!("the final page iteration always returns or errors")
    }
}

impl NetrShareEnumOut {
    fn shares(&self) -> crate::Result<Vec<ShareInfo1>> {
        let ShareEnumUnion::Info1(container) = &*self.info_struct.share_info else {
            return Err(crate::SmbRpcError::InvalidResponseData(
                "NetrShareEnum returned an unsupported share info level",
            ));
        };
        let Some(container) = &**container else {
            return Ok(Vec::new());
        };
        let Some(buffer) = &*container.buffer else {
            return Ok(Vec::new());
        };
        Ok(buffer.iter().map(|entry| (**entry).clone()).collect())
    }

    fn append_to(
        &self,
        result: &mut Vec<ShareInfo1>,
        current_resume_handle: u32,
        seen_resume_handles: &mut HashSet<u32>,
    ) -> crate::Result<Option<u32>> {
        let status = *self.status;
        if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
            return Err(crate::SmbRpcError::InvalidResponseData(
                "NetrShareEnum returned an unsuccessful status",
            ));
        }

        let shares = self.shares()?;
        if *self.total_entries as usize > MAX_SHARE_ENUM_ENTRIES
            || result.len().saturating_add(shares.len()) > MAX_SHARE_ENUM_ENTRIES
        {
            return Err(crate::SmbRpcError::InvalidResponseData(
                "NetrShareEnum returned too many shares",
            ));
        }
        if status == ERROR_MORE_DATA && shares.is_empty() {
            return Err(crate::SmbRpcError::InvalidResponseData(
                "NetrShareEnum returned more data without any shares",
            ));
        }
        result.extend(shares);

        if status == ERROR_SUCCESS {
            return Ok(None);
        }

        let next_resume_handle = self
            .resume_handle
            .as_ref()
            .map(|value| **value)
            .filter(|value| *value != 0)
            .ok_or(crate::SmbRpcError::InvalidResponseData(
                "NetrShareEnum omitted the continuation handle",
            ))?;
        if next_resume_handle == current_resume_handle
            || !seen_resume_handles.insert(next_resume_handle)
        {
            return Err(crate::SmbRpcError::InvalidResponseData(
                "NetrShareEnum repeated the continuation handle",
            ));
        }
        Ok(Some(next_resume_handle))
    }
}

impl<T> super::base::RpcInterface<T> for SrvSvc<T>
where
    T: BoundRpcConnection,
{
    const SYNTAX_ID: DceRpcSyntaxId = DceRpcSyntaxId {
        uuid: make_guid!("4b324fc8-1670-01d3-1278-5a47bf6ee188"),
        version: 3,
    };

    const TRANSFER_SYNTAXES: &'static [DceRpcSyntaxId] =
        &[crate::ndr64::NDR64_SYNTAX_ID, crate::ndr32::NDR32_SYNTAX_ID];

    fn new(bound_pipe: T) -> Self {
        SrvSvc { bound_pipe }
    }
}

#[cfg(test)]
mod test {
    use smb_tests::*;
    use std::{
        collections::VecDeque,
        future::Future,
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
        time::Duration,
    };

    use super::*;

    struct MockBoundRpc {
        syntax: RpcTransferSyntax,
        responses: VecDeque<Vec<u8>>,
        requests: Vec<(u16, Vec<u8>)>,
    }

    impl BoundRpcConnection for MockBoundRpc {
        fn transfer_syntax(&self) -> RpcTransferSyntax {
            self.syntax
        }

        #[maybe_async]
        async fn send_receive_raw(
            &mut self,
            opnum: u16,
            stub_input: &[u8],
        ) -> crate::Result<Vec<u8>> {
            self.requests.push((opnum, stub_input.to_vec()));
            self.responses
                .pop_front()
                .ok_or(crate::SmbRpcError::InvalidResponseData(
                    "mock has no more RPC responses",
                ))
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        struct ThreadWake(std::thread::Thread);
        impl Wake for ThreadWake {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }

            fn wake_by_ref(self: &Arc<Self>) {
                self.0.unpark();
            }
        }

        let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(result) => return result,
                Poll::Pending => std::thread::park_timeout(Duration::from_millis(1)),
            }
        }
    }

    fn from_hex(hex: &str) -> Vec<u8> {
        hex.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_ndr32_string(bytes: &mut Vec<u8>, value: &str) {
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        let units = value
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        push_u32(bytes, units.len() as u32);
        push_u32(bytes, 0);
        push_u32(bytes, units.len() as u32);
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }

    fn encode_ndr32_page(
        entries: &[(&str, u32, Option<&str>)],
        total_entries: u32,
        resume_handle: Option<u32>,
        status: u32,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [1, 1, if entries.is_empty() { 0 } else { 0x0002_0000 }] {
            push_u32(&mut bytes, value);
        }
        if !entries.is_empty() {
            push_u32(&mut bytes, entries.len() as u32);
            push_u32(&mut bytes, 0x0002_0004);
            push_u32(&mut bytes, entries.len() as u32);
            for (index, (_, share_type, remark)) in entries.iter().enumerate() {
                push_u32(&mut bytes, 0x0002_0008 + index as u32 * 8);
                push_u32(&mut bytes, *share_type);
                push_u32(
                    &mut bytes,
                    if remark.is_some() {
                        0x0002_000c + index as u32 * 8
                    } else {
                        0
                    },
                );
            }
            for (name, _, remark) in entries {
                push_ndr32_string(&mut bytes, name);
                if let Some(remark) = remark {
                    push_ndr32_string(&mut bytes, remark);
                }
            }
        }
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        push_u32(&mut bytes, total_entries);
        push_u32(
            &mut bytes,
            if resume_handle.is_some() {
                0x0002_0010
            } else {
                0
            },
        );
        if let Some(resume_handle) = resume_handle {
            push_u32(&mut bytes, resume_handle);
        }
        push_u32(&mut bytes, status);
        bytes
    }

    fn decode_ndr32_page(bytes: &[u8]) -> crate::Result<NetrShareEnumOut> {
        NetrShareEnumIn::deserialize_for(bytes, RpcTransferSyntax::Ndr32)
    }

    fn encode_ndr64_page(
        entries: &[(&str, u32, Option<&str>)],
        total_entries: u32,
        resume_handle: Option<u32>,
        status: u32,
    ) -> Vec<u8> {
        let shares = entries
            .iter()
            .map(|(name, share_type, remark)| ShareInfo1 {
                netname: (*name).parse::<NdrString<u16>>().unwrap().into(),
                share_type: ShareType::from_bytes(share_type.to_le_bytes()).into(),
                remark: remark
                    .unwrap_or_default()
                    .parse::<NdrString<u16>>()
                    .unwrap()
                    .into(),
            })
            .collect::<Vec<_>>();
        let response = NetrShareEnumOut {
            info_struct: ShareEnumStruct {
                share_info: ShareEnumUnion::Info1(
                    ShareInfoContainer::<ShareInfo1> {
                        buffer: NdrPtr::from(NdrArray::from(shares)),
                    }
                    .into(),
                )
                .into(),
            }
            .into(),
            total_entries: total_entries.into(),
            resume_handle: NdrPtr::from(resume_handle).into(),
            status: status.into(),
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        response.write_le(&mut cursor).unwrap();
        cursor.into_inner()
    }

    fn encode_ndr64_empty_with_null_container() -> Vec<u8> {
        let response = NetrShareEnumOut {
            info_struct: ShareEnumStruct {
                share_info: ShareEnumUnion::Info1(NdrPtr::<ShareInfoContainer<ShareInfo1>>::from(
                    None,
                ))
                .into(),
            }
            .into(),
            total_entries: 0.into(),
            resume_handle: NdrPtr::<u32>::from(None).into(),
            status: ERROR_SUCCESS.into(),
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        response.write_le(&mut cursor).unwrap();
        cursor.into_inner()
    }

    fn decode_ndr64_page(bytes: &[u8]) -> crate::Result<NetrShareEnumOut> {
        NetrShareEnumIn::deserialize_for(bytes, RpcTransferSyntax::Ndr64)
    }

    fn share_names(shares: &[ShareInfo1]) -> Vec<String> {
        shares
            .iter()
            .map(|share| {
                String::from_utf16(&share.netname.as_ref().unwrap().data.value)
                    .unwrap()
                    .trim_end_matches('\0')
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn ndr32_share_enum_request_has_32_bit_pointers_and_counts() {
        let input = NetrShareEnumIn {
            server_name: NdrPtr::from(r"\\localhost".parse::<NdrString<u16>>().unwrap()).into(),
            info_struct: ShareEnumStruct {
                share_info: ShareEnumUnion::Info1(NdrPtr::from(ShareInfoContainer::<ShareInfo1> {
                    buffer: NdrPtr::from(None),
                }))
                .into(),
            }
            .into(),
            prefered_maximum_length: u32::MAX.into(),
            resume_handle: NdrPtr::from(0u32).into(),
        };
        assert_eq!(
            input.serialize_for(RpcTransferSyntax::Ndr32),
            from_hex(
                "000002000c000000000000000c0000005c005c006c006f00630061006c0068006f007300740000000100000001000000040002000000000000000000ffffffff1000020000000000"
            )
        );
    }

    #[test]
    fn ndr32_share_enum_response_decodes_share_name_type_and_remark() {
        let mut response = Vec::new();
        for value in [
            1u32,
            1,
            0x0002_0000,
            1,
            0x0002_0004, // one share in the conformant array
            1,
            0x0002_0008,
            0,
            0x0002_000c, // share name pointer, disk type, remark pointer
            6,
            0,
            6, // name conformant-varying string header
        ] {
            response.extend_from_slice(&value.to_le_bytes());
        }
        for unit in "music\0".encode_utf16() {
            response.extend_from_slice(&unit.to_le_bytes());
        }
        for value in [1u32, 0, 1] {
            // empty remark: max_count, offset, actual_count
            response.extend_from_slice(&value.to_le_bytes());
        }
        response.extend_from_slice(&0u16.to_le_bytes()); // empty remark terminator
        response.extend_from_slice(&[0u8; 2]); // align next output argument to 4 bytes
        response.extend_from_slice(&1u32.to_le_bytes()); // total entries
        response.extend_from_slice(&0x0002_0010u32.to_le_bytes()); // resume handle pointer
        response.extend_from_slice(&0u32.to_le_bytes()); // resume handle value
        response.extend_from_slice(&0u32.to_le_bytes()); // success status
        let decoded = NetrShareEnumIn::deserialize_for(&response, RpcTransferSyntax::Ndr32)
            .expect("valid NDR32 share response");
        let ShareEnumUnion::Info1(container) = &*decoded.info_struct.share_info else {
            panic!("share enum response should use info level 1");
        };
        let Some(container) = &**container else {
            panic!("share container should be present");
        };
        let Some(buffer) = &*container.buffer else {
            panic!("share array should be present");
        };
        assert_eq!(buffer.len(), 1);
        assert_eq!(
            buffer[0].netname.as_ref().unwrap().data.value,
            "music\0".encode_utf16().collect::<Vec<_>>()
        );
        assert_eq!(buffer[0].share_type.kind(), ShareKind::Disk);
        assert_eq!(buffer[0].remark.as_ref().unwrap().data.value, vec![0]);
        assert_eq!(*decoded.total_entries, 1);
        assert_eq!(*decoded.status, ERROR_SUCCESS);
    }

    #[test]
    fn ndr32_accepts_both_valid_empty_response_shapes() {
        // A null union container and a present empty container are both valid empty lists.
        for response in [encode_ndr32_page(&[], 0, None, ERROR_SUCCESS), {
            let mut bytes = Vec::new();
            for value in [1u32, 1, 0x0002_0000, 0, 0, 0, 0, 0] {
                push_u32(&mut bytes, value);
            }
            bytes
        }] {
            let decoded = decode_ndr32_page(&response).expect("valid empty share list");
            assert!(decoded.shares().unwrap().is_empty());
            assert_eq!(*decoded.status, ERROR_SUCCESS);
        }
    }

    #[test]
    fn ndr32_preserves_more_data_status_and_resume_handle() {
        let response =
            encode_ndr32_page(&[("music", 0, Some("audio"))], 2, Some(91), ERROR_MORE_DATA);
        let decoded = decode_ndr32_page(&response).expect("ERROR_MORE_DATA is a valid page");
        assert_eq!(*decoded.status, ERROR_MORE_DATA);
        assert_eq!(**decoded.resume_handle.as_ref().unwrap(), 91);
        assert_eq!(share_names(&decoded.shares().unwrap()), ["music"]);
    }

    #[test]
    fn ndr64_preserves_more_data_status_and_resume_handle() {
        let response =
            encode_ndr64_page(&[("music", 0, Some("audio"))], 2, Some(91), ERROR_MORE_DATA);
        let decoded = decode_ndr64_page(&response).expect("valid NDR64 page");
        assert_eq!(*decoded.status, ERROR_MORE_DATA);
        assert_eq!(**decoded.resume_handle.as_ref().unwrap(), 91);
        assert_eq!(share_names(&decoded.shares().unwrap()), ["music"]);
    }

    #[test]
    fn ndr64_accepts_peer_generated_non_null_referent_ids() {
        let mut response = encode_ndr64_page(&[("music", 0, None)], 1, None, ERROR_SUCCESS);
        // The first pointer is the unique SHARE_INFO_CONTAINER pointer. Its referent ID
        // is not required to match this crate's writer-side default value.
        response[16..24].copy_from_slice(&0x1234_5678_9abc_def0u64.to_le_bytes());
        response[32..40].copy_from_slice(&0x1020_3040_5060_7080u64.to_le_bytes());
        let decoded = decode_ndr64_page(&response).expect("valid non-null referent ID");
        assert_eq!(share_names(&decoded.shares().unwrap()), ["music"]);
    }

    #[test]
    fn ndr64_accepts_empty_null_container_and_rejects_every_truncated_prefix() {
        let empty = encode_ndr64_empty_with_null_container();
        let decoded = decode_ndr64_page(&empty).expect("valid empty NDR64 result");
        assert!(decoded.shares().unwrap().is_empty());

        let populated = encode_ndr64_page(&[("music", 0, Some("audio"))], 1, None, ERROR_SUCCESS);
        for end in 0..populated.len() {
            assert!(
                decode_ndr64_page(&populated[..end]).is_err(),
                "accepted NDR64 prefix {end}"
            );
        }
        assert!(decode_ndr64_page(&populated).is_ok());
    }

    #[test]
    fn ndr64_rejects_excessive_count_before_allocating_for_supported_levels() {
        let response = encode_ndr64_page(&[("music", 0, None)], 1, None, ERROR_SUCCESS);
        assert_eq!(u32::from_le_bytes(response[0..4].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(response[8..16].try_into().unwrap()), 1);

        for (level, switch) in [(ShareInfoLevel::Info0 as u32, 0u64), (1, 1)] {
            let mut hostile = response.clone();
            hostile[0..4].copy_from_slice(&level.to_le_bytes());
            hostile[8..16].copy_from_slice(&switch.to_le_bytes());
            hostile[24..28].copy_from_slice(&((MAX_SHARE_ENUM_ENTRIES + 1) as u32).to_le_bytes());
            assert!(
                decode_ndr64_page(&hostile).is_err(),
                "accepted level {level}"
            );
        }
    }

    #[test]
    fn ndr32_rejects_per_string_and_aggregate_string_limits_before_reading_payloads() {
        let mut too_long = Vec::new();
        for value in [1_048_577u32, 0, 1_048_577] {
            push_u32(&mut too_long, value);
        }
        assert!(Ndr32Reader::new(&too_long).utf16_string().is_err());

        let mut one_unit = Vec::new();
        for value in [1u32, 0, 1] {
            push_u32(&mut one_unit, value);
        }
        one_unit.extend_from_slice(&[b'x', 0]);
        let mut reader = Ndr32Reader::new(&one_unit);
        reader.string_units = MAX_SHARE_ENUM_STRING_UNITS;
        assert!(reader.utf16_string().is_err());
    }

    #[test]
    fn ndr32_truncated_response_is_rejected_at_every_byte_boundary() {
        let response =
            encode_ndr32_page(&[("音樂", 0, Some("shared media"))], 1, None, ERROR_SUCCESS);
        for end in 0..response.len() {
            assert!(
                decode_ndr32_page(&response[..end]).is_err(),
                "accepted prefix {end}"
            );
        }
        assert!(decode_ndr32_page(&response).is_ok());
    }

    #[test]
    fn ndr32_rejects_bad_level_discriminant_pointer_and_counts() {
        let valid = encode_ndr32_page(&[("music", 0, Some("audio"))], 1, None, ERROR_SUCCESS);

        let mut bad_level = valid.clone();
        bad_level[0..4].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode_ndr32_page(&bad_level).is_err());

        let mut bad_switch = valid.clone();
        bad_switch[4..8].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode_ndr32_page(&bad_switch).is_err());

        let mut null_name = valid.clone();
        null_name[24..28].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode_ndr32_page(&null_name).is_err());

        let mut missing_buffer = valid.clone();
        missing_buffer[16..20].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode_ndr32_page(&missing_buffer).is_err());

        let mut count_exceeds_max = valid.clone();
        count_exceeds_max[20..24].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode_ndr32_page(&count_exceeds_max).is_err());

        let mut excessive_count = Vec::new();
        for value in [1u32, 1, 0x0002_0000, 65_537, 0x0002_0004, 65_537] {
            push_u32(&mut excessive_count, value);
        }
        assert!(decode_ndr32_page(&excessive_count).is_err());
    }

    #[test]
    fn ndr32_rejects_invalid_string_offsets_terminators_and_utf16() {
        let valid = encode_ndr32_page(&[("music", 0, Some("audio"))], 1, None, ERROR_SUCCESS);
        let name_start = 36;

        let mut bad_offset = valid.clone();
        bad_offset[name_start + 4..name_start + 8].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode_ndr32_page(&bad_offset).is_err());

        let mut bad_name_terminator = valid.clone();
        let name_units = "music".encode_utf16().count() + 1;
        let terminator = name_start + 12 + (name_units - 1) * 2;
        bad_name_terminator[terminator..terminator + 2]
            .copy_from_slice(&(b'X' as u16).to_le_bytes());
        assert!(decode_ndr32_page(&bad_name_terminator).is_err());

        let mut bad_surrogate = encode_ndr32_page(&[("x", 0, None)], 1, None, ERROR_SUCCESS);
        let invalid_unit = 36 + 12;
        bad_surrogate[invalid_unit..invalid_unit + 2].copy_from_slice(&0xd800u16.to_le_bytes());
        assert!(decode_ndr32_page(&bad_surrogate).is_err());

        let mut trailing = valid;
        trailing.extend_from_slice(&[0, 0, 0, 0]);
        assert!(decode_ndr32_page(&trailing).is_err());
    }

    #[test]
    fn share_enum_page_accumulates_and_rejects_invalid_continuations() {
        let first = decode_ndr32_page(&encode_ndr32_page(
            &[("first", 0, None)],
            2,
            Some(7),
            ERROR_MORE_DATA,
        ))
        .unwrap();
        let second = decode_ndr32_page(&encode_ndr32_page(
            &[("second", 0, None)],
            2,
            None,
            ERROR_SUCCESS,
        ))
        .unwrap();

        let mut result = Vec::new();
        let mut seen = HashSet::new();
        let cursor = first.append_to(&mut result, 0, &mut seen).unwrap().unwrap();
        assert_eq!(cursor, 7);
        assert_eq!(
            second.append_to(&mut result, cursor, &mut seen).unwrap(),
            None
        );
        assert_eq!(share_names(&result), ["first", "second"]);

        let no_cursor = decode_ndr32_page(&encode_ndr32_page(
            &[("first", 0, None)],
            2,
            None,
            ERROR_MORE_DATA,
        ))
        .unwrap();
        assert!(
            no_cursor
                .append_to(&mut Vec::new(), 0, &mut HashSet::new())
                .is_err()
        );

        let zero_cursor = decode_ndr32_page(&encode_ndr32_page(
            &[("first", 0, None)],
            2,
            Some(0),
            ERROR_MORE_DATA,
        ))
        .unwrap();
        assert!(
            zero_cursor
                .append_to(&mut Vec::new(), 0, &mut HashSet::new())
                .is_err()
        );

        let repeated_cursor = decode_ndr32_page(&encode_ndr32_page(
            &[("first", 0, None)],
            2,
            Some(7),
            ERROR_MORE_DATA,
        ))
        .unwrap();
        assert!(
            repeated_cursor
                .append_to(&mut Vec::new(), 7, &mut HashSet::new())
                .is_err()
        );

        let empty_more_data =
            decode_ndr32_page(&encode_ndr32_page(&[], 1, Some(8), ERROR_MORE_DATA)).unwrap();
        assert!(
            empty_more_data
                .append_to(&mut Vec::new(), 0, &mut HashSet::new())
                .is_err()
        );

        let unexpected_status = decode_ndr32_page(&encode_ndr32_page(&[], 0, None, 5)).unwrap();
        assert!(
            unexpected_status
                .append_to(&mut Vec::new(), 0, &mut HashSet::new())
                .is_err()
        );

        let excessive_total = decode_ndr32_page(&encode_ndr32_page(
            &[],
            (MAX_SHARE_ENUM_ENTRIES + 1) as u32,
            None,
            ERROR_SUCCESS,
        ))
        .unwrap();
        assert!(
            excessive_total
                .append_to(&mut Vec::new(), 0, &mut HashSet::new())
                .is_err()
        );
    }

    #[test]
    fn netr_share_enum_requests_and_accumulates_multiple_ndr32_pages() {
        let mock = MockBoundRpc {
            syntax: RpcTransferSyntax::Ndr32,
            responses: VecDeque::from([
                encode_ndr32_page(&[("first", 0, None)], 2, Some(55), ERROR_MORE_DATA),
                encode_ndr32_page(&[("second", 0, None)], 2, None, ERROR_SUCCESS),
            ]),
            requests: Vec::new(),
        };
        let mut service = <SrvSvc<MockBoundRpc> as RpcInterface<MockBoundRpc>>::new(mock);
        let shares = block_on(service.netr_share_enum("\\\\server")).unwrap();
        assert_eq!(share_names(&shares), ["first", "second"]);

        let requests = &service.bound_pipe.requests;
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|(opnum, _)| *opnum == NetrShareEnumIn::OPNUM)
        );
        let initial_cursor = &requests[0].1[requests[0].1.len() - 4..];
        let resumed_cursor = &requests[1].1[requests[1].1.len() - 4..];
        assert_eq!(u32::from_le_bytes(initial_cursor.try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(resumed_cursor.try_into().unwrap()), 55);
    }

    #[test]
    fn netr_share_enum_requests_and_accumulates_multiple_ndr64_pages() {
        let mock = MockBoundRpc {
            syntax: RpcTransferSyntax::Ndr64,
            responses: VecDeque::from([
                encode_ndr64_page(&[("first", 0, None)], 2, Some(55), ERROR_MORE_DATA),
                encode_ndr64_page(&[("second", 0, None)], 2, None, ERROR_SUCCESS),
            ]),
            requests: Vec::new(),
        };
        let mut service = <SrvSvc<MockBoundRpc> as RpcInterface<MockBoundRpc>>::new(mock);
        let shares = block_on(service.netr_share_enum("\\\\server")).unwrap();
        assert_eq!(share_names(&shares), ["first", "second"]);

        let requests = &service.bound_pipe.requests;
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|(opnum, _)| *opnum == NetrShareEnumIn::OPNUM)
        );
        let initial_cursor = &requests[0].1[requests[0].1.len() - 4..];
        let resumed_cursor = &requests[1].1[requests[1].1.len() - 4..];
        assert_eq!(u32::from_le_bytes(initial_cursor.try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(resumed_cursor.try_into().unwrap()), 55);
    }

    #[test]
    fn netr_share_enum_stops_after_the_page_limit() {
        let responses = (0..MAX_SHARE_ENUM_PAGES)
            .map(|cursor| {
                encode_ndr32_page(
                    &[("share", 0, None)],
                    MAX_SHARE_ENUM_PAGES as u32 + 1,
                    Some(cursor as u32 + 1),
                    ERROR_MORE_DATA,
                )
            })
            .collect();
        let mock = MockBoundRpc {
            syntax: RpcTransferSyntax::Ndr32,
            responses,
            requests: Vec::new(),
        };
        let mut service = <SrvSvc<MockBoundRpc> as RpcInterface<MockBoundRpc>>::new(mock);
        assert!(block_on(service.netr_share_enum("\\\\server")).is_err());
        assert_eq!(service.bound_pipe.requests.len(), MAX_SHARE_ENUM_PAGES);
    }

    test_binrw_read! {
        struct NetrShareEnumOut {
                info_struct: ShareEnumStruct {
                    share_info: ShareEnumUnion::Info1(
                        ShareInfoContainer::<ShareInfo1> {
                            buffer: Into::<NdrArray<ShareInfo1>>::into(vec![
                                ShareInfo1 {
                                    netname: "ADMIN$".parse::<NdrString<u16>>().unwrap().into(),
                                    share_type: ShareType::new().with_special(true).into(),
                                    remark: "Remote Admin"
                                        .parse::<NdrString<u16>>()
                                        .unwrap()
                                        .into(),
                                },
                                ShareInfo1 {
                                    netname: "C$".parse::<NdrString<u16>>().unwrap().into(),
                                    share_type: ShareType::new().with_special(true).into(),
                                    remark: "Default share"
                                        .parse::<NdrString<u16>>()
                                        .unwrap()
                                        .into(),
                                },
                                ShareInfo1 {
                                    netname: "IPC$".parse::<NdrString<u16>>().unwrap().into(),
                                    share_type: ShareType::new()
                                        .with_special(true)
                                        .with_kind(ShareKind::IPC)
                                        .into(),
                                    remark: "Remote IPC".parse::<NdrString<u16>>().unwrap().into(),
                                },
                                ShareInfo1 {
                                    netname: "LocalAdminShare"
                                        .parse::<NdrString<u16>>()
                                        .unwrap()
                                        .into(),
                                    share_type: ShareType::new().into(),
                                    remark: "".parse::<NdrString<u16>>().unwrap().into(),
                                },
                                ShareInfo1 {
                                    netname: "MyShare".parse::<NdrString<u16>>().unwrap().into(),
                                    share_type: ShareType::new().into(),
                                    remark: "".parse::<NdrString<u16>>().unwrap().into(),
                                },
                                ShareInfo1 {
                                    netname: "PublicShare"
                                        .parse::<NdrString<u16>>()
                                        .unwrap()
                                        .into(),
                                    share_type: ShareType::new().into(),
                                    remark: "".parse::<NdrString<u16>>().unwrap().into(),
                                },
                            ])
                            .into()
                        }
                        .into()
                    )
                    .into()
                }
                .into(),
                total_entries: 6.into(),
                resume_handle: NdrPtr::<u32>::from(None).into(),
                status: 0.into(),
            } => "010000000000000001000000000000000000020000000000060000000000000000000200000000000600000000000000000002000000000000000080000000000000020000000000000002000000000000000080000000000000020000000000000002000000000003000080000000000000020000000000000002000000000000000000000000000000020000000000000002000000000000000000000000000000020000000000000002000000000000000000000000000000020000000000070000000000000000000000000000000700000000000000410044004d0049004e002400000000000d0000000000000000000000000000000d00000000000000520065006d006f00740065002000410064006d0069006e00000000000000000003000000000000000000000000000000030000000000000043002400000000000e0000000000000000000000000000000e00000000000000440065006600610075006c007400200073006800610072006500000000000000050000000000000000000000000000000500000000000000490050004300240000000000000000000b0000000000000000000000000000000b00000000000000520065006d006f00740065002000490050004300000000001000000000000000000000000000000010000000000000004c006f00630061006c00410064006d0069006e0053006800610072006500000001000000000000000000000000000000010000000000000000000000000000000800000000000000000000000000000008000000000000004d00790053006800610072006500000001000000000000000000000000000000010000000000000000000000000000000c0000000000000000000000000000000c000000000000005000750062006c00690063005300680061007200650000000100000000000000000000000000000001000000000000000000000006000000000000000000000000000000"
    }

    smb_tests::test_binrw_write! {
        struct NetrShareEnumIn {
            server_name: Into::<NdrPtr<_>>::into(r"\\localhost".parse::<NdrString<u16>>().unwrap())
                .into(),
            info_struct: ShareEnumStruct {
                share_info: ShareEnumUnion::Info1(NdrPtr::from(ShareInfoContainer::<ShareInfo1> {
                    buffer: NdrPtr::from(None),
                }))
                .into(),
            }
            .into(),
            prefered_maximum_length: 4294967295.into(),
            resume_handle: NdrPtr::<u32>::from(None).into(),
        } => "00000200000000000c0000000000000000000000000000000c000000000000005c005c006c006f00630061006c0068006f0073007400000001000000000000000100000000000000000002000000000000000000000000000000000000000000ffffffff000000000000000000000000"
    }
}
