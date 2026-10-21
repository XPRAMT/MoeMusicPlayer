# SMB RPC

This crate contains the MS-RPC implementation used by SMB for operations such as SRVSVC share enumeration.
SRVSVC share enumeration supports both NDR64 and NDR32; the SMB pipe selects the syntax accepted by the server.
Other RPC interfaces default to NDR64.

For now, RPC structures and functions are manually implemented - not derived from an IDL file.

> This crate is a part of the `smb-rs` project
