//! The tunnel: WireGuard (boringtun) over a UDP socket, carrying the packets of a TCP/IP stack
//! (smoltcp) whose TCP connections the proxy hands out as [`VirtualStream`]s.
//!
//! One task drives everything: it polls the stack, seals what the stack sends and opens what the
//! peer sends. The session with the peer (the UDP socket, its handshakes and keepalives) opens
//! only when the stack has something to send, and closes after [`IDLE`] without a connection or
//! a packet: the VPN is not kept up while nothing uses it. The streams change their sockets under the same lock and wake the task. Two
//! things make the throughput (ADR 0037): large receive buffers (the TCP window, scaled, is what
//! one round trip can carry: 64 KB caps a 40 ms tunnel at 15 Mb/s), and no timer in the data's
//! path (ACKs at once, no Nagle: Windows' timers tick every 15.6 ms).

use std::collections::{HashMap, VecDeque};
use std::future::poll_fn;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use smoltcp::iface::{Config as IfaceConfig, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{self, Device, DeviceCapabilities, Medium};
use smoltcp::socket::{dns, tcp};
use smoltcp::wire::{DnsQueryType, HardwareAddress, IpAddress, IpCidr};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::UdpSocket;
use tokio::sync::Notify;

use crate::{Config, Error};

/// A TCP connection's receive buffer: its window, about 800 Mb/s over a 40 ms tunnel.
const RX_BUFFER: usize = 4 << 20;
/// Its send buffer: requests are small.
const TX_BUFFER: usize = 512 << 10;
/// The largest UDP datagram.
const MAX_DATAGRAM: usize = 65_536;
/// How often WireGuard's timers run (handshakes, keepalives), as boringtun asks.
const TIMER_TICK: Duration = Duration::from_millis(250);
/// How long a closed connection may take to end before it is dropped.
const LINGER: Duration = Duration::from_secs(30);
/// A connection that gets no answer for this long is given up.
const TCP_TIMEOUT: Duration = Duration::from_secs(20);
/// Before the endpoint's name resolves, tried again this often.
const RESOLVE_RETRY: Duration = Duration::from_secs(5);
/// Names resolved through the tunnel are kept this long.
const DNS_KEPT: Duration = Duration::from_secs(300);
/// Queries to the tunnel's name server at once.
const DNS_QUERIES: usize = 8;
/// Without a connection through the tunnel nor a packet for this long, the session with the peer
/// closes; the next connection opens it again (the endpoint's name, a handshake: a round trip).
pub const IDLE: Duration = Duration::from_secs(5 * 60);

/// A running tunnel and its local proxy. Dropping it stops both.
pub struct Tunnel {
  shared: Arc<Shared>,
  proxy_url: String,
  tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// What the tunnel is doing, for the settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
  /// How long ago the last handshake with the peer succeeded; `None` before the first (the peer
  /// has not answered: wrong key, endpoint unreachable).
  pub handshake: Option<Duration>,
  /// Why the endpoint cannot be reached, when known (its name does not resolve, no network).
  pub error: Option<String>,
  /// The endpoint's address, once its name resolved.
  pub endpoint: Option<SocketAddr>,
  /// How long the session with the peer has been open; `None` while it is closed (nothing went
  /// through the tunnel yet, or nothing for [`IDLE`]).
  pub connected: Option<Duration>,
}

impl Tunnel {
  /// Starts the tunnel on `runtime`. Its proxy's address is known at once (its socket opens
  /// here); the session with the peer opens at the first connection (the endpoint's name, a
  /// handshake), which waits for it, and closes after [`IDLE`] unused.
  pub fn start(config: Config, runtime: &tokio::runtime::Handle) -> Result<Self, Error> {
    Self::start_with_idle(config, runtime, IDLE)
  }

  /// [`Tunnel::start`], its session closing after `idle` unused.
  pub fn start_with_idle(
    config: Config,
    runtime: &tokio::runtime::Handle,
    idle: Duration,
  ) -> Result<Self, Error> {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let proxy_url = format!("http://{}:{secret}@127.0.0.1:{port}", crate::proxy::USER);
    let shared = Arc::new(Shared::new(&config, idle));
    let tasks = vec![
      runtime.spawn(drive(config.clone(), shared.clone())),
      runtime.spawn(crate::proxy::serve(
        listener,
        shared.clone(),
        config,
        secret,
      )),
    ];
    Ok(Self {
      shared,
      proxy_url,
      tasks,
    })
  }

  /// The local proxy, `http://user:secret@127.0.0.1:port`: for reqwest's `Proxy::all` and mpv's
  /// `http-proxy`. It holds the proxy's secret: never log it.
  pub fn proxy_url(&self) -> &str {
    &self.proxy_url
  }

  /// The tunnel's state now.
  pub fn status(&self) -> Status {
    let status = lock(&self.shared.status);
    Status {
      handshake: status.handshake.map(|at| at.elapsed()),
      error: status.error.clone(),
      endpoint: status.endpoint,
      connected: status.since.map(|at| at.elapsed()),
    }
  }
}

impl Drop for Tunnel {
  fn drop(&mut self) {
    // The proxy's connections end too (`proxy::serve`).
    self.shared.stop.send_replace(true);
    for task in &self.tasks {
      task.abort();
    }
  }
}

/// What the driving task shares with the streams.
pub(crate) struct Shared {
  net: Mutex<Net>,
  /// Wakes the driving task: a stream sent data, read some (the window opened) or closed.
  wake: Notify,
  status: Mutex<StatusInner>,
  /// Set when the tunnel is dropped.
  stop: tokio::sync::watch::Sender<bool>,
  /// How long the session stays open unused.
  idle: Duration,
  /// Names resolved through the tunnel, and when.
  names: Mutex<HashMap<String, (IpAddr, Instant)>>,
}

#[derive(Default)]
struct StatusInner {
  handshake: Option<Instant>,
  error: Option<String>,
  endpoint: Option<SocketAddr>,
  /// When the session opened, while it is open.
  since: Option<Instant>,
}

/// The TCP/IP stack.
struct Net {
  iface: Interface,
  sockets: SocketSet<'static>,
  device: Queues,
  /// Connections whose stream went: removed once they end, or after [`LINGER`].
  dropped: Vec<(SocketHandle, Instant)>,
  /// The name server inside the tunnel (`DNS`), when the peer routes it.
  dns: Option<SocketHandle>,
  next_port: u16,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
  // A panic under the lock leaves the stack as it was between two calls: still usable.
  mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
  fn new(config: &Config, idle: Duration) -> Self {
    let mut device = Queues {
      mtu: usize::from(config.mtu),
      ..Queues::default()
    };
    let mut iface_config = IfaceConfig::new(HardwareAddress::Ip);
    iface_config.random_seed = uuid::Uuid::new_v4().as_u64_pair().0;
    let mut iface = Interface::new(iface_config, &mut device, smoltcp::time::Instant::now());
    iface.update_ip_addrs(|addrs| {
      for &(ip, prefix) in &config.addresses {
        if addrs
          .push(IpCidr::new(IpAddress::from(ip), prefix))
          .is_err()
        {
          tracing::warn!("WireGuard: too many addresses, {ip}/{prefix} left out");
        }
      }
    });
    // Everything goes to the peer: with no link layer, the gateway is only a name.
    for &(ip, _) in &config.addresses {
      let added = match ip {
        std::net::IpAddr::V4(ip) => iface.routes_mut().add_default_ipv4_route(ip).is_ok(),
        std::net::IpAddr::V6(ip) => iface.routes_mut().add_default_ipv6_route(ip).is_ok(),
      };
      if !added {
        tracing::warn!("WireGuard: no route through {ip}");
      }
    }
    let mut sockets = SocketSet::new(Vec::new());
    let servers: Vec<IpAddress> = config
      .dns
      .iter()
      .filter(|&&ip| config.routes(ip))
      .map(|&ip| IpAddress::from(ip))
      .collect();
    let dns = (!servers.is_empty()).then(|| {
      let queries: Vec<Option<dns::DnsQuery>> = (0..DNS_QUERIES).map(|_| None).collect();
      sockets.add(dns::Socket::new(&servers, queries))
    });
    Self {
      net: Mutex::new(Net {
        iface,
        sockets,
        device,
        dropped: Vec::new(),
        dns,
        next_port: 49_152,
      }),
      wake: Notify::new(),
      stop: tokio::sync::watch::Sender::new(false),
      idle,
      names: Mutex::new(HashMap::new()),
      status: Mutex::new(StatusInner::default()),
    }
  }

  /// Opens a TCP connection to `remote` through the tunnel; [`VirtualStream::established`]
  /// waits for it.
  pub(crate) fn connect(self: &Arc<Self>, remote: SocketAddr) -> io::Result<VirtualStream> {
    let mut socket = tcp::Socket::new(
      tcp::SocketBuffer::new(vec![0; RX_BUFFER]),
      tcp::SocketBuffer::new(vec![0; TX_BUFFER]),
    );
    socket.set_nagle_enabled(false);
    socket.set_ack_delay(None);
    socket.set_timeout(Some(smoltcp::time::Duration::from_millis(
      TCP_TIMEOUT.as_millis() as u64,
    )));
    let handle = {
      let mut net = lock(&self.net);
      let port = net.next_port;
      net.next_port = if port == u16::MAX { 49_152 } else { port + 1 };
      let Net { iface, .. } = &mut *net;
      socket
        .connect(iface.context(), remote, port)
        .map_err(|e| io::Error::new(io::ErrorKind::AddrNotAvailable, e.to_string()))?;
      net.sockets.add(socket)
    };
    self.wake.notify_one();
    Ok(VirtualStream {
      shared: self.clone(),
      handle,
    })
  }

  /// `name`'s IPv4 address from the name server inside the tunnel (`DNS`); `None` without one,
  /// or when it does not know the name (smoltcp gives up after 10 s).
  pub(crate) async fn lookup(&self, name: &str) -> Option<IpAddr> {
    let key = name.to_ascii_lowercase();
    if let Some(&(ip, at)) = lock(&self.names).get(&key)
      && at.elapsed() < DNS_KEPT
    {
      return Some(ip);
    }
    let query = {
      let mut net = lock(&self.net);
      let Net {
        iface,
        sockets,
        dns,
        ..
      } = &mut *net;
      let handle = (*dns)?;
      let query = sockets
        .get_mut::<dns::Socket>(handle)
        .start_query(iface.context(), name, DnsQueryType::A)
        .ok()?;
      Query {
        shared: self,
        socket: handle,
        query,
        done: false,
      }
    };
    self.wake.notify_one();
    let ip = query.result().await;
    if let Some(ip) = ip {
      lock(&self.names).insert(key, (ip, Instant::now()));
    }
    ip
  }

  /// Resolves once the tunnel is dropped.
  pub(crate) async fn stopped(&self) {
    let mut stop = self.stop.subscribe();
    // An error: the sender went, which is a stop too.
    let _ = stop.wait_for(|stopped| *stopped).await;
  }

  fn set_error(&self, error: Option<String>) {
    lock(&self.status).error = error;
  }
}

impl Net {
  /// Runs the stack: what came in, what the sockets send. Returns what to send to the peer and
  /// when to run it again at the latest.
  fn poll(&mut self) -> (VecDeque<Vec<u8>>, Option<Duration>) {
    let now = smoltcp::time::Instant::now();
    self.iface.poll(now, &mut self.device, &mut self.sockets);
    let sockets = &mut self.sockets;
    self.dropped.retain(|&(handle, at)| {
      let state = sockets.get::<tcp::Socket>(handle).state();
      let ended = matches!(state, tcp::State::Closed | tcp::State::TimeWait);
      if ended || at.elapsed() > LINGER {
        sockets.remove(handle);
        false
      } else {
        true
      }
    });
    let delay = self
      .iface
      .poll_delay(now, &self.sockets)
      .map(|d| Duration::from_micros(d.total_micros()));
    (std::mem::take(&mut self.device.tx), delay)
  }

  /// Whether a TCP connection goes through the tunnel (open, or still closing).
  fn connections(&self) -> bool {
    self
      .sockets
      .iter()
      .any(|(handle, _)| Some(handle) != self.dns)
  }
}

/// Drives the tunnel until it is dropped: the stack alone while nothing goes through it, a
/// session with the peer while something does.
async fn drive(config: Config, shared: Arc<Shared>) {
  loop {
    let first = traffic(&shared).await;
    session(&config, &shared, first).await;
    let mut status = lock(&shared.status);
    status.since = None;
    status.handshake = None;
    drop(status);
    tracing::debug!("WireGuard: the session closed, unused");
  }
}

/// Runs the stack without a session until it sends something (a connection, a name to
/// resolve): returns those packets.
async fn traffic(shared: &Shared) -> VecDeque<Vec<u8>> {
  loop {
    let (outgoing, delay) = lock(&shared.net).poll();
    if !outgoing.is_empty() {
      return outgoing;
    }
    match delay {
      Some(delay) => {
        tokio::select! {
          () = shared.wake.notified() => {}
          () = tokio::time::sleep(delay) => {}
        }
      }
      None => shared.wake.notified().await,
    }
  }
}

/// One session with the peer, from `first` (the packets that open it) until nothing used it for
/// [`Shared::idle`].
async fn session(config: &Config, shared: &Shared, first: VecDeque<Vec<u8>>) {
  let mut used = Instant::now();
  lock(&shared.status).since = Some(used);
  tracing::debug!("WireGuard: the session opens");
  let udp = loop {
    match bind(&config.endpoint).await {
      Ok(udp) => {
        let mut status = lock(&shared.status);
        status.error = None;
        status.endpoint = udp.peer_addr().ok();
        drop(status);
        break udp;
      }
      Err(e) => {
        tracing::warn!("WireGuard endpoint: {e}");
        shared.set_error(Some(e.to_string()));
        tokio::time::sleep(RESOLVE_RETRY).await;
        // Meanwhile the connections waiting may give up (what they send is sent again).
        let mut net = lock(&shared.net);
        net.poll();
        if net.connections() {
          used = Instant::now();
        } else if used.elapsed() >= shared.idle {
          return;
        }
      }
    }
  };
  let mut tunn = Tunn::new(
    StaticSecret::from(config.private_key),
    PublicKey::from(config.peer_key),
    config.preshared_key,
    config.keepalive,
    uuid::Uuid::new_v4().as_u64_pair().0 as u32,
    None,
  );
  let mut out = vec![0; MAX_DATAGRAM + 64];
  let mut datagram = vec![0; MAX_DATAGRAM];
  // Queued by boringtun until the handshake ends, which the first one starts.
  let mut pending = Some(first);
  let mut ticked = Instant::now();
  loop {
    let (mut outgoing, delay) = {
      let mut net = lock(&shared.net);
      let polled = net.poll();
      if net.connections() {
        used = Instant::now();
      }
      polled
    };
    if let Some(mut first) = pending.take() {
      first.append(&mut outgoing);
      outgoing = first;
    }
    if !outgoing.is_empty() {
      used = Instant::now();
    }
    for packet in outgoing {
      match tunn.encapsulate(&packet, &mut out) {
        TunnResult::WriteToNetwork(sealed) => send(&udp, sealed).await,
        TunnResult::Err(e) => tracing::debug!("WireGuard: cannot seal a packet: {e:?}"),
        _ => {}
      }
    }
    if used.elapsed() >= shared.idle {
      return;
    }
    if ticked.elapsed() >= TIMER_TICK {
      ticked = Instant::now();
      match tunn.update_timers(&mut out) {
        TunnResult::WriteToNetwork(packet) => send(&udp, packet).await,
        TunnResult::Err(e) => tracing::debug!("WireGuard timers: {e:?}"),
        _ => {}
      }
      lock(&shared.status).handshake = tunn
        .time_since_last_handshake()
        .and_then(|age| Instant::now().checked_sub(age));
    }
    let tick = TIMER_TICK.saturating_sub(ticked.elapsed());
    let wait = delay.map_or(tick, |delay| delay.min(tick));
    tokio::select! {
      received = udp.recv(&mut datagram) => match received {
        Ok(n) => {
          let mut inbound = Vec::new();
          open(&mut tunn, &udp, &datagram[..n], &mut out, &mut inbound).await;
          // What else already arrived, before the stack runs.
          for _ in 0..256 {
            match udp.try_recv(&mut datagram) {
              Ok(n) => open(&mut tunn, &udp, &datagram[..n], &mut out, &mut inbound).await,
              Err(_) => break,
            }
          }
          if !inbound.is_empty() {
            used = Instant::now();
            lock(&shared.net).device.rx.extend(inbound);
          }
        }
        // Windows reports an ICMP "port unreachable" on the next receive: the peer is down.
        Err(e) => {
          tracing::debug!("WireGuard: receive: {e}");
          tokio::time::sleep(Duration::from_millis(50)).await;
        }
      },
      () = shared.wake.notified() => {}
      () = tokio::time::sleep(wait) => {}
    }
  }
}

/// A UDP socket connected to the endpoint, its name resolved (IPv4 first).
async fn bind(endpoint: &str) -> io::Result<UdpSocket> {
  let addrs: Vec<SocketAddr> = tokio::net::lookup_host(endpoint).await?.collect();
  let remote = addrs
    .iter()
    .find(|a| a.is_ipv4())
    .or(addrs.first())
    .copied()
    .ok_or_else(|| {
      io::Error::new(
        io::ErrorKind::NotFound,
        "the endpoint's name has no address",
      )
    })?;
  let local: SocketAddr = if remote.is_ipv4() {
    (Ipv4Addr::UNSPECIFIED, 0).into()
  } else {
    (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
  };
  let udp = UdpSocket::bind(local).await?;
  udp.connect(remote).await?;
  Ok(udp)
}

async fn send(udp: &UdpSocket, packet: &[u8]) {
  if let Err(e) = udp.send(packet).await {
    tracing::debug!("WireGuard: send: {e}");
  }
}

/// Opens a datagram from the peer: its packets go to `inbound`, its answers (handshake, cookie,
/// the packets queued before the handshake) to the peer.
async fn open(
  tunn: &mut Tunn,
  udp: &UdpSocket,
  datagram: &[u8],
  out: &mut [u8],
  inbound: &mut Vec<Vec<u8>>,
) {
  let mut input = datagram;
  loop {
    match tunn.decapsulate(None, input, out) {
      TunnResult::WriteToNetwork(packet) => {
        send(udp, packet).await;
        // boringtun's queue: called again with nothing until it is empty.
        input = &[];
      }
      TunnResult::WriteToTunnelV4(packet, _) | TunnResult::WriteToTunnelV6(packet, _) => {
        inbound.push(packet.to_vec());
        return;
      }
      TunnResult::Err(e) => {
        tracing::debug!("WireGuard: cannot open a packet: {e:?}");
        return;
      }
      TunnResult::Done => return,
    }
  }
}

/// A query to the tunnel's name server, cancelled when dropped before its answer.
struct Query<'a> {
  shared: &'a Shared,
  socket: SocketHandle,
  query: dns::QueryHandle,
  done: bool,
}

impl Query<'_> {
  async fn result(mut self) -> Option<IpAddr> {
    poll_fn(|cx| {
      let mut net = lock(&self.shared.net);
      let socket = net.sockets.get_mut::<dns::Socket>(self.socket);
      match socket.get_query_result(self.query) {
        Ok(addresses) => {
          self.done = true;
          Poll::Ready(addresses.first().map(|&ip| IpAddr::from(ip)))
        }
        Err(dns::GetQueryResultError::Pending) => {
          socket.register_query_waker(self.query, cx.waker());
          Poll::Pending
        }
        Err(dns::GetQueryResultError::Failed) => {
          self.done = true;
          Poll::Ready(None)
        }
      }
    })
    .await
  }
}

impl Drop for Query<'_> {
  fn drop(&mut self) {
    if !self.done {
      lock(&self.shared.net)
        .sockets
        .get_mut::<dns::Socket>(self.socket)
        .cancel_query(self.query);
    }
  }
}

/// The stack's link: packets in from the peer, packets out to it.
#[derive(Default)]
struct Queues {
  rx: VecDeque<Vec<u8>>,
  tx: VecDeque<Vec<u8>>,
  mtu: usize,
}

struct RxToken(Vec<u8>);
struct TxToken<'a>(&'a mut VecDeque<Vec<u8>>);

impl phy::RxToken for RxToken {
  fn consume<R, F>(self, f: F) -> R
  where
    F: FnOnce(&[u8]) -> R,
  {
    f(&self.0)
  }
}

impl phy::TxToken for TxToken<'_> {
  fn consume<R, F>(self, len: usize, f: F) -> R
  where
    F: FnOnce(&mut [u8]) -> R,
  {
    let mut packet = vec![0; len];
    let result = f(&mut packet);
    self.0.push_back(packet);
    result
  }
}

impl Device for Queues {
  type RxToken<'a> = RxToken;
  type TxToken<'a> = TxToken<'a>;

  fn receive(
    &mut self,
    _: smoltcp::time::Instant,
  ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
    let packet = self.rx.pop_front()?;
    Some((RxToken(packet), TxToken(&mut self.tx)))
  }

  fn transmit(&mut self, _: smoltcp::time::Instant) -> Option<Self::TxToken<'_>> {
    Some(TxToken(&mut self.tx))
  }

  fn capabilities(&self) -> DeviceCapabilities {
    let mut caps = DeviceCapabilities::default();
    caps.medium = Medium::Ip;
    caps.max_transmission_unit = self.mtu;
    caps
  }
}

/// A TCP connection through the tunnel. Dropping it closes the connection.
pub(crate) struct VirtualStream {
  shared: Arc<Shared>,
  handle: SocketHandle,
}

impl VirtualStream {
  /// Waits until the connection is up; an error when the other side refuses it or does not
  /// answer ([`TCP_TIMEOUT`]).
  pub(crate) async fn established(&self) -> io::Result<()> {
    poll_fn(|cx| {
      let mut net = lock(&self.shared.net);
      let socket = net.sockets.get_mut::<tcp::Socket>(self.handle);
      match socket.state() {
        tcp::State::Established => Poll::Ready(Ok(())),
        tcp::State::SynSent | tcp::State::SynReceived => {
          socket.register_send_waker(cx.waker());
          Poll::Pending
        }
        _ => Poll::Ready(Err(io::Error::new(
          io::ErrorKind::ConnectionRefused,
          "the server refused the connection, or did not answer",
        ))),
      }
    })
    .await
  }

  fn with<R>(&self, f: impl FnOnce(&mut tcp::Socket<'static>) -> R) -> R {
    let result = {
      let mut net = lock(&self.shared.net);
      f(net.sockets.get_mut::<tcp::Socket>(self.handle))
    };
    self.shared.wake.notify_one();
    result
  }
}

impl AsyncRead for VirtualStream {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<io::Result<()>> {
    let mut net = lock(&self.shared.net);
    let socket = net.sockets.get_mut::<tcp::Socket>(self.handle);
    if socket.can_recv() {
      let read = socket.recv_slice(buf.initialize_unfilled());
      drop(net);
      // The window opened: the stack may say so.
      self.shared.wake.notify_one();
      return Poll::Ready(match read {
        Ok(n) => {
          buf.advance(n);
          Ok(())
        }
        Err(e) => Err(io::Error::new(
          io::ErrorKind::ConnectionReset,
          e.to_string(),
        )),
      });
    }
    match socket.state() {
      tcp::State::SynSent | tcp::State::SynReceived => {}
      _ if socket.may_recv() => {}
      // The other side finished (FIN), or the connection is gone: the end.
      _ => return Poll::Ready(Ok(())),
    }
    socket.register_recv_waker(cx.waker());
    Poll::Pending
  }
}

impl AsyncWrite for VirtualStream {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    data: &[u8],
  ) -> Poll<io::Result<usize>> {
    let mut net = lock(&self.shared.net);
    let socket = net.sockets.get_mut::<tcp::Socket>(self.handle);
    if socket.can_send() {
      let sent = socket.send_slice(data);
      drop(net);
      self.shared.wake.notify_one();
      return Poll::Ready(
        sent.map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e.to_string())),
      );
    }
    match socket.state() {
      tcp::State::SynSent | tcp::State::SynReceived => {}
      _ if socket.may_send() => {}
      _ => {
        return Poll::Ready(Err(io::Error::new(
          io::ErrorKind::BrokenPipe,
          "the connection is closed",
        )));
      }
    }
    socket.register_send_waker(cx.waker());
    Poll::Pending
  }

  fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
    Poll::Ready(Ok(()))
  }

  fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
    self.with(|socket| socket.close());
    Poll::Ready(Ok(()))
  }
}

impl Drop for VirtualStream {
  fn drop(&mut self) {
    {
      let mut net = lock(&self.shared.net);
      net.sockets.get_mut::<tcp::Socket>(self.handle).close();
      net.dropped.push((self.handle, Instant::now()));
    }
    self.shared.wake.notify_one();
  }
}
