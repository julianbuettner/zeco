# ZeCo
Invite to your Zellij session, peer to peer, end-to-end encrypted.

## Installation
For now, you can simply install the cli via `cargo`.
```
cargo install zeco
```

## Requirements
- Same zellij version, for guest and host.
- Internet connection.

## Usage
To invite peers to your Zellij session, simply run
```
zeco host
```
from within your session. This will generate a join command looking like this:
```
zeco join <host-id> <secret>
```
Send this command to your guest and let them join in.
Running the join command then looks like this:
```
zellij attach <remote-session>
```

## How does it work?
Zeco uses [Iroh](https://www.iroh.computer/) to establish a end-to-end encrypted
peer to peer connection. Iroh bootstraps this connection via their servers
and the protocol bases on QUIC.
The host id contains a generated public key of the host.
This way, the guest can authenticate the host. The host expects the guest to provide
the pre shared secret. This way, the host can trust the guest.

## Notes
- Currently only one guest can join
- Currently only one join can occur, the guest can not re-join with the same data.
- Currently zellij of the guest freezes, if the host terminates the connection.
