//! NDR32 transfer syntax used by SRVSVC share enumeration.

use smb_dtyp::make_guid;

use crate::pdu::DceRpcSyntaxId;

/// The standard Network Data Representation transfer syntax (NDR 2.0).
pub const NDR32_SYNTAX_ID: DceRpcSyntaxId = DceRpcSyntaxId {
    uuid: make_guid!("8a885d04-1ceb-11c9-9fe8-08002b104860"),
    version: 2,
};
