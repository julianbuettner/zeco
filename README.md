# Zeco
Invite to your Zellij session, peer to peer, end-to-end encrypted.

Ideal for situations like pair programming in Neovim.

## Installation
For now, you can simply install the cli via `cargo`.
```
cargo install zeco
```

## Requirements
- Same zellij version, for guest and host
- Internet connection

## Usage
To invite peers to your Zellij session, simply run
```
zeco host
```
from within your zellij session.
This will generate a join command looking like this:
```
zeco join <host-id> <secret>
```
Send this command to your guest and let them join in.
Running the join command then outputs the zellij attach command:
```
zellij attach <remote-session>
```

## How does it work?
Zellij has a server client architecture. This usually works over a linux socket,
but in theory (and in practice) access to the socket can be proxied.

Zeco uses [Iroh](https://www.iroh.computer/) to establish a end-to-end encrypted
peer to peer connection. Iroh bootstraps this connection via their servers
and the protocol is based on QUIC.
The host id basically is a generated public key of the host.
This way, the guest can authenticate the host. The host expects the guest to provide
the pre shared secret. This way, the host can authenticate the guest.

## Notes
- Currently only one guest can join
- Currently only one join can occur, the guest can not re-join with the same data.
- Currently zellij of the guest freezes, if the host terminates the connection.
- The size of the zellij window will be defined by smaller terminal window.
- Zeco does not work for tmux, because tmux transfers things like file descriptors via the socket.
