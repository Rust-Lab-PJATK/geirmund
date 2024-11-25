# Why

Packet is a straightforward library designed to wrap our protobuf with header information, enabling us to distinguish where the protobuf payload begins and ends. This solution exists because protobuf lacks inherent start and end markers.

The protocol is simple and somewhat resembles HTTP in structure, though it is not request-response based.

### Example:
```
RLPG/0.1.0
200

[200 bytes of protobuf]
```

The message begins with `RLPG/[version]`:
- **RLPG** stands for "Rust Lab PJATK Geirmund"
- **version** corresponds to its value in `Cargo.toml`

Following this, a Unix-style newline (LF) is used (unlike HTTP, which uses a Windows-style CRLF), and then a number indicating the length of the protobuf message.

Finally, the protobuf message itself is included in raw bytes.

## IMPORTANT

Note that the protocol does not include markers to distinguish between request and response messages. This is because the protocol **is not designed for a request-response architecture**. Its sole purpose is to indicate where the protobuf message starts and ends.
If you want to include the info about the request or response, you should include it in the protobuf message itself.
