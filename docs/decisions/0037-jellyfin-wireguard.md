# 0037: A Jellyfin server behind WireGuard, in userspace

- Status: accepted
- Date: 2026-11-11

## Context

The user's Jellyfin server can sit behind a WireGuard VPN. With the official client, the whole PC
joins the VPN (a network adapter, administrator rights, every program's traffic routed). The user
asked for a WireGuard configuration in the Jellyfin settings, for every HTTP call and every video
of that space only.

There are two kinds of traffic: the space's API client (`jellyfin-api`, reqwest: pages, images,
reports, trickplay) and mpv, which opens the Direct Play file (and external subtitles) itself.

Spike (`spikes/jellyfin-wg`, 2026-11-11, the user's server on the LAN, its endpoint on the same
network):
- onetun (boringtun 0.6 + smoltcp behind a TCP port forward): handshake and first answer in about
  140 ms, but API round trips of 32 to 48 ms (its loop waits on timers, which tick every 15.6 ms on
  Windows) and **15 Mb/s**: each virtual TCP socket has a 64 KB buffer, so one round trip carries
  at most 64 KB. A 4K remux needs 40 to 80 Mb/s.
- The same with 4 MB buffers (a patched copy): about 400 Mb/s.
- Our own glue (boringtun 0.7.1 + smoltcp 0.14, below): API round trips of 1 to 2 ms, 300 to
  400 Mb/s, first byte of a seek in 45 to 160 ms. A request to an address outside `AllowedIPs` is
  refused.

Options considered:
- The official tunnel (wireguard-nt, wintun): needs a driver and administrator rights, and
  routes the whole PC. Rejected.
- onetun as a library: fixed 64 KB buffers, a port forward per destination, and HTTPS would need
  the server's name pointed at the forward. Rejected.
- gotatun (Mullvad's fork of boringtun): its crypto needs aws-lc-rs or ring, and it is built for a
  TUN device. boringtun's `noise` module alone does what is needed.
- A local proxy for mpv that rewrites URLs and relays ranges (like `twitch::Proxy`): more code,
  and the stream URLs would change. An HTTP proxy is simpler: reqwest takes one (`Proxy::all`) and
  so does mpv (`http-proxy`, FFmpeg's `CONNECT` for HTTPS), and the URLs stay the server's.

## Decision

- A crate `wireguard` (no GPUI): `Config::parse` reads a wg-quick file (one `[Peer]`; `DNS`'s
  search domains and the `Pre`/`Post` commands are ignored), `Tunnel::start` runs it on the
  network runtime:
  - boringtun's `Tunn` over a UDP socket connected to the endpoint (its name resolved by
    Windows, IPv4 first, tried again every 5 s), WireGuard's timers every 250 ms;
  - the VPN only while used (the user asked, 2026-11-11): the proxy and the stack are always
    there (the loopback only), the session with the peer (the UDP socket, its handshakes and
    keepalives) opens when the stack first sends something (a connection's SYN, a DNS query) and
    closes after `wireguard::IDLE` (5 minutes) without a TCP connection through the tunnel nor a
    packet. A connection kept open counts as use: mpv's, while a video is open even paused;
    reqwest's idle pooled ones close after 90 s on their own;
  - smoltcp (IP medium, default routes to the peer) with 4 MB receive and 512 KB send buffers per
    connection, no Nagle, ACKs at once; one task drives the stack and the tunnel, and the
    connections' streams change their sockets under the same lock and wake it;
  - a local HTTP proxy on 127.0.0.1, port chosen by Windows: `CONNECT` for HTTPS (TLS runs end to
    end), absolute URLs for plain HTTP. It asks for Basic credentials holding a random secret (a
    407 with its challenge without them), so another program on the PC cannot use the tunnel, and
    it carries only addresses within the peer's `AllowedIPs`;
  - names are resolved by the configuration's `DNS` server through the tunnel first (smoltcp's
    DNS socket, A records, kept 5 minutes), when the peer routes it, as wg-quick does, then by
    Windows: a server's name often resolves to the VPN's address only on the VPN's DNS (the
    user's `mediaserver.astal.space` is behind Cloudflare publicly, 192.168.1.50 on the VPN).
- `jellyfin_api::Client::with_proxy`: every request of the space's client through the proxy.
  `player::StartOptions::http_proxy`: mpv's `http-proxy`, for the file and its subtitles.
- The space (`vpn.rs`): the configuration is imported from a `.conf` file in its settings (shown
  signed out too, since a server behind a VPN needs it to sign in). It is stored in the
  Credential Manager (`jellyfin|wireguard`, rewritten without comments or ignored keys:
  `Config::to_wg_quick`), never in the settings file. `[jellyfin] vpn` turns it on or off. A change
  restarts the tunnel and rebuilds the session's client: the pages reload on Home. The section
  shows the endpoint, the addresses, the routes and the last handshake, or "on standby" while the
  session is closed.
- Nothing reaches the server before Jellyfin is used, so that launching Peeby does not connect
  the VPN: the sidebar's libraries and user are asked at the space's first render, not at
  sign-in, and the settings page asks the server's name and version only when that opens no
  session (direct, or the session already open).
- The keys, the stored text and the proxy's URL (its secret) never reach a log or an error
  message; `Config`'s and `Client`'s `Debug` leave them out.

## Consequences

- No driver, no administrator rights, nothing else on the PC goes through the VPN.
- The endpoint's name is resolved by Windows, before the tunnel exists: it must point at the VPN
  server's own address. A name behind Cloudflare's proxy (as the user's `vpn.astal.space` turned
  out to be, 2026-11-11) never answers a handshake: the settings say so after 10 s without one,
  and show the address the name resolved to.
- An opt-in mpv log file may hold mpv's options, the proxy's URL among them: the secret is
  random per launch and the proxy listens on the loopback only.
- Measured on a LAN. Over the Internet the server's upload and the tunnel's round trip set the
  throughput. With 4 MB windows, a 100 ms tunnel still carries over 300 Mb/s.
- The first request after a launch or a standby pays the session's opening: the endpoint's name,
  then a handshake (one round trip), before its TCP connection.
- A connection opened before the first handshake waits for it (boringtun queues its packets).
  The first answer took about 1 s in some runs: either the system's name lookup or a lost SYN
  sent again by smoltcp after 1 s.
- Revisit if boringtun stops being maintained (gotatun keeps the same `noise` API).
