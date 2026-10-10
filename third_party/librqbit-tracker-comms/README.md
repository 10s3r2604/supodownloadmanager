This package is a dependency of [rqbit](https://github.com/ikatson/rqbit) torrent client.
It can be used by itself too. See more [at the rqbit Github page](https://github.com/ikatson/rqbit).

## SDM changes

This is `librqbit-tracker-comms` 9.0.1 from [rqbit](https://github.com/ikatson/rqbit)
(Apache-2.0, see `LICENSE`), used by Supo Download Manager through
`[patch.crates-io]` in the workspace `Cargo.toml`. One change, in
`src/tracker_comms_udp.rs`: when the computer has no IPv6, the UDP tracker
client binds an IPv4 socket instead of failing, so BitTorrent still starts.
Remove this folder once upstream has the same fallback.
