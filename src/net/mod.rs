pub mod ne2k;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::{pci, task, timer};

const ETHERNET_HEADER_LEN: usize = 14;
const IPV4_HEADER_LEN: usize = 20;
const UDP_HEADER_LEN: usize = 8;
const TCP_HEADER_LEN: usize = 20;
const ARP_PACKET_LEN: usize = 28;
const MAX_FRAME_BYTES: usize = 1536;
const MAX_FRAMES_PER_POLL: usize = 16;
const ARP_RESOLVE_TIMEOUT_TICKS: u32 = 12;
const ARP_CACHE_SIZE: usize = 8;
const MAX_TCP_CONNECTIONS: usize = 8;
const DNS_MAX_HOST_LEN: usize = 96;
const UDP_MAX_PAYLOAD: usize = MAX_FRAME_BYTES - ETHERNET_HEADER_LEN - IPV4_HEADER_LEN - UDP_HEADER_LEN;

const ETH_TYPE_IPV4: u16 = 0x0800;
const ETH_TYPE_ARP: u16 = 0x0806;
const IP_PROTO_UDP: u8 = 17;
const IP_PROTO_TCP: u8 = 6;

const ARP_HW_ETHERNET: u16 = 1;
const ARP_OP_REQUEST: u16 = 1;
const ARP_OP_REPLY: u16 = 2;

const TCP_FLAG_FIN: u16 = 0x01;
const TCP_FLAG_SYN: u16 = 0x02;
const TCP_FLAG_RST: u16 = 0x04;
const TCP_FLAG_ACK: u16 = 0x10;

const BROADCAST_MAC: [u8; 6] = [0xFF; 6];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError {
    NotInitialized,
    NoDevice,
    Unsupported,
    InvalidAddress,
    Timeout,
    Busy,
    BufferTooSmall,
}

impl NetError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotInitialized => "network not initialized",
            Self::NoDevice => "no supported network device",
            Self::Unsupported => "operation unsupported",
            Self::InvalidAddress => "invalid network address",
            Self::Timeout => "network timeout",
            Self::Busy => "network busy",
            Self::BufferTooSmall => "buffer too small",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Addr {
    octets: [u8; 4],
}

impl Ipv4Addr {
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self {
            octets: [a, b, c, d],
        }
    }

    pub const fn octets(self) -> [u8; 4] {
        self.octets
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpHandle {
    pub id: u32,
    pub remote_ip: Ipv4Addr,
    pub remote_port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetStats {
    pub initialized: bool,
    pub nic_present: bool,
    pub nic_name: &'static str,
    pub pci_devices: usize,
    pub polls: u32,
    pub tx_packets: u32,
    pub rx_packets: u32,
    pub arp_requests: u32,
    pub arp_hits: u32,
    pub dns_queries: u32,
    pub dns_success: u32,
    pub tcp_connects: u32,
    pub tcp_established: u32,
    pub last_poll_tick: u32,
}

#[derive(Clone, Copy)]
struct ArpEntry {
    valid: bool,
    ip: Ipv4Addr,
    mac: [u8; 6],
    last_seen_tick: u32,
}

impl ArpEntry {
    const fn empty() -> Self {
        Self {
            valid: false,
            ip: Ipv4Addr::new(0, 0, 0, 0),
            mac: [0; 6],
            last_seen_tick: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct DnsPending {
    active: bool,
    id: u16,
    local_port: u16,
    host: [u8; DNS_MAX_HOST_LEN],
    host_len: usize,
    result: Option<Ipv4Addr>,
}

impl DnsPending {
    const fn empty() -> Self {
        Self {
            active: false,
            id: 0,
            local_port: 0,
            host: [0; DNS_MAX_HOST_LEN],
            host_len: 0,
            result: None,
        }
    }
}

#[derive(Clone, Copy)]
struct UdpPending {
    active: bool,
    local_port: u16,
    remote_port: u16,
    remote_ip: Ipv4Addr,
    response: [u8; UDP_MAX_PAYLOAD],
    response_len: usize,
}

impl UdpPending {
    const fn empty() -> Self {
        Self {
            active: false,
            local_port: 0,
            remote_port: 0,
            remote_ip: Ipv4Addr::new(0, 0, 0, 0),
            response: [0; UDP_MAX_PAYLOAD],
            response_len: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TcpState {
    Closed,
    SynSent,
    Established,
    Reset,
}

#[derive(Clone, Copy)]
struct TcpConnection {
    used: bool,
    id: u32,
    state: TcpState,
    local_port: u16,
    remote_port: u16,
    remote_ip: Ipv4Addr,
    seq: u32,
    ack: u32,
    last_activity_tick: u32,
}

impl TcpConnection {
    const fn empty() -> Self {
        Self {
            used: false,
            id: 0,
            state: TcpState::Closed,
            local_port: 0,
            remote_port: 0,
            remote_ip: Ipv4Addr::new(0, 0, 0, 0),
            seq: 0,
            ack: 0,
            last_activity_tick: 0,
        }
    }
}

struct NetStackState {
    local_mac: [u8; 6],
    local_ip: Ipv4Addr,
    netmask: Ipv4Addr,
    gateway: Ipv4Addr,
    dns_server: Ipv4Addr,
    arp_cache: [ArpEntry; ARP_CACHE_SIZE],
    next_ip_id: u16,
    next_ephemeral_port: u16,
    next_dns_id: u16,
    dns_pending: DnsPending,
    udp_pending: UdpPending,
    tcp_connections: [TcpConnection; MAX_TCP_CONNECTIONS],
}

static ONLINE: AtomicBool = AtomicBool::new(false);
static NEXT_TCP_HANDLE_ID: AtomicU32 = AtomicU32::new(1);

static mut STATS: NetStats = NetStats {
    initialized: false,
    nic_present: false,
    nic_name: "none",
    pci_devices: 0,
    polls: 0,
    tx_packets: 0,
    rx_packets: 0,
    arp_requests: 0,
    arp_hits: 0,
    dns_queries: 0,
    dns_success: 0,
    tcp_connects: 0,
    tcp_established: 0,
    last_poll_tick: 0,
};

static mut STATE: Option<NetStackState> = None;

pub fn init() -> Result<(), NetError> {
    let pci_devices = pci::scan();
    let nic = ne2k::probe();

    unsafe {
        STATS.pci_devices = pci_devices;
        STATS.initialized = true;
        STATS.nic_present = nic.is_some();
        STATS.nic_name = if nic.is_some() { "ne2k-pci" } else { "none" };
    }

    let Some(device) = nic else {
        ONLINE.store(false, Ordering::Release);
        unsafe {
            STATE = None;
        }
        return Err(NetError::NoDevice);
    };

    ne2k::init_pci(device)?;
    let mac = ne2k::mac_address().ok_or(NetError::Unsupported)?;

    unsafe {
        STATE = Some(NetStackState {
            local_mac: mac,
            local_ip: Ipv4Addr::new(10, 0, 2, 15),
            netmask: Ipv4Addr::new(255, 255, 255, 0),
            gateway: Ipv4Addr::new(10, 0, 2, 2),
            dns_server: Ipv4Addr::new(10, 0, 2, 3),
            arp_cache: [ArpEntry::empty(); ARP_CACHE_SIZE],
            next_ip_id: 1,
            next_ephemeral_port: 40000,
            next_dns_id: 1,
            dns_pending: DnsPending::empty(),
            udp_pending: UdpPending::empty(),
            tcp_connections: [TcpConnection::empty(); MAX_TCP_CONNECTIONS],
        });
    }

    ONLINE.store(true, Ordering::Release);
    Ok(())
}

pub fn is_online() -> bool {
    ONLINE.load(Ordering::Acquire)
}

pub fn poll(now_ticks: u32) {
    unsafe {
        STATS.polls = STATS.polls.wrapping_add(1);
        STATS.last_poll_tick = now_ticks;
    }

    if !is_online() {
        return;
    }

    ne2k::poll(now_ticks);

    let mut processed = 0usize;
    while processed < MAX_FRAMES_PER_POLL {
        let mut frame = [0u8; MAX_FRAME_BYTES];
        let Some(len) = ne2k::recv_frame(&mut frame) else {
            break;
        };
        processed += 1;

        if len < ETHERNET_HEADER_LEN {
            continue;
        }

        unsafe {
            STATS.rx_packets = STATS.rx_packets.wrapping_add(1);
        }
        handle_ethernet_frame(&frame[..len], now_ticks);
    }
}

pub fn stats() -> NetStats {
    unsafe { STATS }
}

pub fn local_ipv4() -> Option<Ipv4Addr> {
    unsafe { STATE.as_ref().map(|state| state.local_ip) }
}

pub fn gateway_ipv4() -> Option<Ipv4Addr> {
    unsafe { STATE.as_ref().map(|state| state.gateway) }
}

pub fn dns_ipv4() -> Option<Ipv4Addr> {
    unsafe { STATE.as_ref().map(|state| state.dns_server) }
}

pub fn mac_address() -> Option<[u8; 6]> {
    unsafe { STATE.as_ref().map(|state| state.local_mac) }
}

pub fn parse_ipv4_literal(input: &str) -> Option<Ipv4Addr> {
    parse_ipv4(input)
}

pub fn udp_request(dst_ip: Ipv4Addr, dst_port: u16, payload: &[u8]) -> Result<u16, NetError> {
    if !is_online() {
        return Err(NetError::NotInitialized);
    }
    if dst_port == 0 {
        return Err(NetError::InvalidAddress);
    }
    if payload.len() > UDP_MAX_PAYLOAD {
        return Err(NetError::BufferTooSmall);
    }

    let src_port = unsafe {
        let Some(state) = STATE.as_mut() else {
            return Err(NetError::NotInitialized);
        };
        if state.udp_pending.local_port != 0 {
            return Err(NetError::Busy);
        }

        let src_port = allocate_ephemeral_port(state);
        state.udp_pending = UdpPending {
            active: true,
            local_port: src_port,
            remote_port: dst_port,
            remote_ip: dst_ip,
            response: [0; UDP_MAX_PAYLOAD],
            response_len: 0,
        };
        src_port
    };

    if let Err(error) = send_udp_packet_nonblocking(dst_ip, dst_port, src_port, payload) {
        unsafe {
            if let Some(state) = STATE.as_mut() {
                if state.udp_pending.local_port == src_port {
                    state.udp_pending = UdpPending::empty();
                }
            }
        }
        return Err(error);
    }

    Ok(src_port)
}

pub fn udp_request_poll(request_port: u16, response: &mut [u8]) -> Result<Option<usize>, NetError> {
    if request_port == 0 {
        return Err(NetError::InvalidAddress);
    }

    let pending = unsafe {
        let Some(state) = STATE.as_ref() else {
            return Err(NetError::NotInitialized);
        };
        state.udp_pending
    };

    if pending.local_port != request_port {
        return Err(NetError::InvalidAddress);
    }

    if pending.active {
        return Ok(None);
    }

    let copied = pending.response_len.min(response.len());
    response[..copied].copy_from_slice(&pending.response[..copied]);

    unsafe {
        if let Some(state) = STATE.as_mut() {
            if state.udp_pending.local_port == request_port {
                state.udp_pending = UdpPending::empty();
            }
        }
    }

    Ok(Some(copied))
}

pub fn udp_request_cancel(request_port: u16) -> Result<(), NetError> {
    if request_port == 0 {
        return Err(NetError::InvalidAddress);
    }

    unsafe {
        let Some(state) = STATE.as_mut() else {
            return Err(NetError::NotInitialized);
        };

        if state.udp_pending.local_port == request_port {
            state.udp_pending = UdpPending::empty();
            return Ok(());
        }
    }

    Err(NetError::InvalidAddress)
}

pub fn udp_exchange(
    dst_ip: Ipv4Addr,
    dst_port: u16,
    payload: &[u8],
    response: &mut [u8],
    timeout_ticks: u32,
) -> Result<usize, NetError> {
    let src_port = udp_request(dst_ip, dst_port, payload)?;

    let start = timer::ticks();
    loop {
        let now = timer::ticks();
        poll(now);

        if let Some(copied) = udp_request_poll(src_port, response)? {
            return Ok(copied);
        }

        if now.wrapping_sub(start) > timeout_ticks {
            let _ = udp_request_cancel(src_port);
            return Err(NetError::Timeout);
        }

        task::sleep_ticks(1);
    }
}

pub fn dns_resolve(host: &str) -> Result<Ipv4Addr, NetError> {
    if let Some(ip) = parse_ipv4(host) {
        return Ok(ip);
    }

    if !is_online() {
        return Err(NetError::NotInitialized);
    }

    if host.is_empty() || host.len() >= DNS_MAX_HOST_LEN {
        return Err(NetError::InvalidAddress);
    }

    unsafe {
        STATS.dns_queries = STATS.dns_queries.wrapping_add(1);
    }

    let (dns_server, id, src_port) = unsafe {
        let Some(state) = STATE.as_mut() else {
            return Err(NetError::NotInitialized);
        };

        if state.dns_pending.active {
            return Err(NetError::Busy);
        }

        let id = state.next_dns_id;
        state.next_dns_id = state.next_dns_id.wrapping_add(1).max(1);
        let src_port = allocate_ephemeral_port(state);

        let mut host_buf = [0u8; DNS_MAX_HOST_LEN];
        let host_len = copy_ascii(&mut host_buf, host.as_bytes());
        state.dns_pending = DnsPending {
            active: true,
            id,
            local_port: src_port,
            host: host_buf,
            host_len,
            result: None,
        };

        (state.dns_server, id, src_port)
    };

    let mut question = [0u8; 512];
    let payload_len = build_dns_query_packet(id, host, &mut question)?;
    send_udp_packet(dns_server, 53, src_port, &question[..payload_len])?;

    let start = timer::ticks();
    let timeout = 400u32;
    loop {
        let now = timer::ticks();
        poll(now);

        let result = unsafe {
            let Some(state) = STATE.as_mut() else {
                return Err(NetError::NotInitialized);
            };

            if state.dns_pending.active && state.dns_pending.id == id {
                state.dns_pending.result
            } else {
                None
            }
        };

        if let Some(ip) = result {
            unsafe {
                if let Some(state) = STATE.as_mut() {
                    state.dns_pending.active = false;
                    state.dns_pending.result = None;
                    STATS.dns_success = STATS.dns_success.wrapping_add(1);
                }
            }
            return Ok(ip);
        }

        if now.wrapping_sub(start) > timeout {
            unsafe {
                if let Some(state) = STATE.as_mut() {
                    if state.dns_pending.active && state.dns_pending.id == id {
                        state.dns_pending.active = false;
                        state.dns_pending.result = None;
                    }
                }
            }
            return Err(NetError::Timeout);
        }

        task::sleep_ticks(1);
    }
}

pub fn tcp_connect(ip: Ipv4Addr, port: u16) -> Result<TcpHandle, NetError> {
    if !is_online() {
        return Err(NetError::NotInitialized);
    }
    if port == 0 {
        return Err(NetError::InvalidAddress);
    }

    unsafe {
        STATS.tcp_connects = STATS.tcp_connects.wrapping_add(1);
    }

    let conn_index = unsafe {
        let Some(state) = STATE.as_mut() else {
            return Err(NetError::NotInitialized);
        };

        let Some(index) = state.tcp_connections.iter().position(|conn| !conn.used) else {
            return Err(NetError::Busy);
        };

        let local_port = allocate_ephemeral_port(state);
        let seq_seed = timer::ticks().wrapping_mul(1103515245).wrapping_add(12345);
        let id = NEXT_TCP_HANDLE_ID.fetch_add(1, Ordering::AcqRel);

        state.tcp_connections[index] = TcpConnection {
            used: true,
            id,
            state: TcpState::SynSent,
            local_port,
            remote_port: port,
            remote_ip: ip,
            seq: seq_seed,
            ack: 0,
            last_activity_tick: timer::ticks(),
        };

        index
    };

    send_tcp_segment(conn_index, TCP_FLAG_SYN, &[])?;

    let start = timer::ticks();
    let timeout = 600u32;
    loop {
        let now = timer::ticks();
        poll(now);

        let state_now = unsafe {
            let Some(state) = STATE.as_ref() else {
                return Err(NetError::NotInitialized);
            };
            state.tcp_connections[conn_index]
        };

        match state_now.state {
            TcpState::Established => {
                return Ok(TcpHandle {
                    id: state_now.id,
                    remote_ip: state_now.remote_ip,
                    remote_port: state_now.remote_port,
                });
            }
            TcpState::Reset | TcpState::Closed => {
                unsafe {
                    if let Some(state) = STATE.as_mut() {
                        state.tcp_connections[conn_index] = TcpConnection::empty();
                    }
                }
                return Err(NetError::Unsupported);
            }
            TcpState::SynSent => {}
        }

        if now.wrapping_sub(start) > timeout {
            unsafe {
                if let Some(state) = STATE.as_mut() {
                    state.tcp_connections[conn_index] = TcpConnection::empty();
                }
            }
            return Err(NetError::Timeout);
        }

        task::sleep_ticks(1);
    }
}

pub fn format_ipv4(ip: Ipv4Addr, out: &mut [u8; 16]) -> usize {
    let octets = ip.octets();
    let mut len = 0usize;
    for (index, octet) in octets.iter().enumerate() {
        if index > 0 && len < out.len() {
            out[len] = b'.';
            len += 1;
        }
        len += push_u8_decimal(out, len, *octet);
    }
    len.min(out.len())
}

fn handle_ethernet_frame(frame: &[u8], now_ticks: u32) {
    if frame.len() < ETHERNET_HEADER_LEN {
        return;
    }

    let eth_type = u16::from_be_bytes([frame[12], frame[13]]);
    let payload = &frame[ETHERNET_HEADER_LEN..];

    match eth_type {
        ETH_TYPE_ARP => handle_arp_frame(payload, now_ticks),
        ETH_TYPE_IPV4 => handle_ipv4_frame(payload, now_ticks),
        _ => {}
    }
}

fn handle_arp_frame(payload: &[u8], now_ticks: u32) {
    if payload.len() < ARP_PACKET_LEN {
        return;
    }

    let hardware = u16::from_be_bytes([payload[0], payload[1]]);
    let protocol = u16::from_be_bytes([payload[2], payload[3]]);
    let hlen = payload[4];
    let plen = payload[5];
    let opcode = u16::from_be_bytes([payload[6], payload[7]]);

    if hardware != ARP_HW_ETHERNET || protocol != ETH_TYPE_IPV4 || hlen != 6 || plen != 4 {
        return;
    }

    let sender_mac = [
        payload[8], payload[9], payload[10], payload[11], payload[12], payload[13],
    ];
    let sender_ip = Ipv4Addr::new(payload[14], payload[15], payload[16], payload[17]);
    let target_ip = Ipv4Addr::new(payload[24], payload[25], payload[26], payload[27]);

    arp_cache_insert(sender_ip, sender_mac, now_ticks);

    let local_ip = unsafe {
        let Some(state) = STATE.as_ref() else {
            return;
        };
        state.local_ip
    };

    if opcode == ARP_OP_REQUEST && target_ip == local_ip {
        let _ = send_arp_reply(sender_mac, sender_ip);
    }
}

fn handle_ipv4_frame(payload: &[u8], now_ticks: u32) {
    if payload.len() < IPV4_HEADER_LEN {
        return;
    }

    let version_ihl = payload[0];
    if (version_ihl >> 4) != 4 {
        return;
    }

    let ihl = ((version_ihl & 0x0F) as usize) * 4;
    if ihl < IPV4_HEADER_LEN || ihl > payload.len() {
        return;
    }

    let total_len = u16::from_be_bytes([payload[2], payload[3]]) as usize;
    if total_len < ihl || total_len > payload.len() {
        return;
    }

    let protocol = payload[9];
    let src_ip = Ipv4Addr::new(payload[12], payload[13], payload[14], payload[15]);
    let dst_ip = Ipv4Addr::new(payload[16], payload[17], payload[18], payload[19]);

    let local_ip = unsafe {
        let Some(state) = STATE.as_ref() else {
            return;
        };
        state.local_ip
    };

    if dst_ip != local_ip {
        return;
    }

    let body = &payload[ihl..total_len];
    match protocol {
        IP_PROTO_UDP => handle_udp_segment(src_ip, dst_ip, body, now_ticks),
        IP_PROTO_TCP => handle_tcp_segment(src_ip, dst_ip, body, now_ticks),
        _ => {}
    }
}

fn handle_udp_segment(src_ip: Ipv4Addr, _dst_ip: Ipv4Addr, payload: &[u8], _now_ticks: u32) {
    if payload.len() < UDP_HEADER_LEN {
        return;
    }

    let src_port = u16::from_be_bytes([payload[0], payload[1]]);
    let dst_port = u16::from_be_bytes([payload[2], payload[3]]);
    let udp_len = u16::from_be_bytes([payload[4], payload[5]]) as usize;

    if udp_len < UDP_HEADER_LEN || udp_len > payload.len() {
        return;
    }

    let body = &payload[UDP_HEADER_LEN..udp_len];

    let pending = unsafe {
        let Some(state) = STATE.as_ref() else {
            return;
        };
        state.dns_pending
    };

    if pending.active && src_port == 53 && dst_port == pending.local_port {
        if let Some(ip) = parse_dns_response(pending.id, body) {
            unsafe {
                if let Some(state) = STATE.as_mut() {
                    if state.dns_pending.active
                        && state.dns_pending.id == pending.id
                        && state.dns_pending.local_port == pending.local_port
                    {
                        state.dns_pending.result = Some(ip);
                    }
                }
            }
        }
    }

    unsafe {
        if let Some(state) = STATE.as_mut() {
            if state.udp_pending.active
                && dst_port == state.udp_pending.local_port
                && src_port == state.udp_pending.remote_port
                && src_ip == state.udp_pending.remote_ip
            {
                let copy_len = body.len().min(state.udp_pending.response.len());
                state.udp_pending.response[..copy_len].copy_from_slice(&body[..copy_len]);
                state.udp_pending.response_len = copy_len;
                state.udp_pending.active = false;
            }
        }
    }

}

fn handle_tcp_segment(src_ip: Ipv4Addr, _dst_ip: Ipv4Addr, payload: &[u8], now_ticks: u32) {
    if payload.len() < TCP_HEADER_LEN {
        return;
    }

    let src_port = u16::from_be_bytes([payload[0], payload[1]]);
    let dst_port = u16::from_be_bytes([payload[2], payload[3]]);
    let seq = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
    let ack = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);
    let data_offset = ((payload[12] >> 4) as usize) * 4;
    if data_offset < TCP_HEADER_LEN || data_offset > payload.len() {
        return;
    }

    let flags = payload[13] as u16;
    let body = &payload[data_offset..];

    let conn_index = unsafe {
        let Some(state) = STATE.as_ref() else {
            return;
        };
        state
            .tcp_connections
            .iter()
            .position(|conn| {
                conn.used
                    && conn.remote_ip == src_ip
                    && conn.remote_port == src_port
                    && conn.local_port == dst_port
            })
    };

    let Some(index) = conn_index else {
        return;
    };

    let mut send_ack = false;
    unsafe {
        let Some(state) = STATE.as_mut() else {
            return;
        };
        let conn = &mut state.tcp_connections[index];
        conn.last_activity_tick = now_ticks;

        if (flags & TCP_FLAG_RST) != 0 {
            conn.state = TcpState::Reset;
            return;
        }

        match conn.state {
            TcpState::SynSent => {
                if (flags & TCP_FLAG_SYN) != 0 && (flags & TCP_FLAG_ACK) != 0 && ack == conn.seq {
                    conn.ack = seq.wrapping_add(1);
                    conn.state = TcpState::Established;
                    send_ack = true;
                    STATS.tcp_established = STATS.tcp_established.wrapping_add(1);
                }
            }
            TcpState::Established => {
                if !body.is_empty() {
                    conn.ack = seq.wrapping_add(body.len() as u32);
                    send_ack = true;
                }
                if (flags & TCP_FLAG_FIN) != 0 {
                    conn.ack = conn.ack.wrapping_add(1);
                    conn.state = TcpState::Closed;
                    send_ack = true;
                }
            }
            TcpState::Closed | TcpState::Reset => {}
        }
    }

    if send_ack {
        let _ = send_tcp_segment(index, TCP_FLAG_ACK, &[]);
    }
}

fn send_arp_request(target_ip: Ipv4Addr) -> Result<(), NetError> {
    let (local_mac, local_ip) = unsafe {
        let Some(state) = STATE.as_ref() else {
            return Err(NetError::NotInitialized);
        };
        (state.local_mac, state.local_ip)
    };

    let mut arp = [0u8; ARP_PACKET_LEN];
    arp[0..2].copy_from_slice(&ARP_HW_ETHERNET.to_be_bytes());
    arp[2..4].copy_from_slice(&ETH_TYPE_IPV4.to_be_bytes());
    arp[4] = 6;
    arp[5] = 4;
    arp[6..8].copy_from_slice(&ARP_OP_REQUEST.to_be_bytes());
    arp[8..14].copy_from_slice(&local_mac);
    arp[14..18].copy_from_slice(&local_ip.octets());
    arp[18..24].fill(0);
    arp[24..28].copy_from_slice(&target_ip.octets());

    unsafe {
        STATS.arp_requests = STATS.arp_requests.wrapping_add(1);
    }
    send_ethernet_frame(BROADCAST_MAC, ETH_TYPE_ARP, &arp)
}

fn send_arp_reply(target_mac: [u8; 6], target_ip: Ipv4Addr) -> Result<(), NetError> {
    let (local_mac, local_ip) = unsafe {
        let Some(state) = STATE.as_ref() else {
            return Err(NetError::NotInitialized);
        };
        (state.local_mac, state.local_ip)
    };

    let mut arp = [0u8; ARP_PACKET_LEN];
    arp[0..2].copy_from_slice(&ARP_HW_ETHERNET.to_be_bytes());
    arp[2..4].copy_from_slice(&ETH_TYPE_IPV4.to_be_bytes());
    arp[4] = 6;
    arp[5] = 4;
    arp[6..8].copy_from_slice(&ARP_OP_REPLY.to_be_bytes());
    arp[8..14].copy_from_slice(&local_mac);
    arp[14..18].copy_from_slice(&local_ip.octets());
    arp[18..24].copy_from_slice(&target_mac);
    arp[24..28].copy_from_slice(&target_ip.octets());

    send_ethernet_frame(target_mac, ETH_TYPE_ARP, &arp)
}

fn send_udp_packet(
    dst_ip: Ipv4Addr,
    dst_port: u16,
    src_port: u16,
    payload: &[u8],
) -> Result<(), NetError> {
    if payload.len() > UDP_MAX_PAYLOAD {
        return Err(NetError::BufferTooSmall);
    }

    let udp_len = UDP_HEADER_LEN + payload.len();
    let mut segment = [0u8; MAX_FRAME_BYTES];

    segment[0..2].copy_from_slice(&src_port.to_be_bytes());
    segment[2..4].copy_from_slice(&dst_port.to_be_bytes());
    segment[4..6].copy_from_slice(&(udp_len as u16).to_be_bytes());
    segment[6..8].copy_from_slice(&0u16.to_be_bytes());
    segment[8..8 + payload.len()].copy_from_slice(payload);

    send_ipv4_packet(dst_ip, IP_PROTO_UDP, &segment[..udp_len])
}

fn send_udp_packet_nonblocking(
    dst_ip: Ipv4Addr,
    dst_port: u16,
    src_port: u16,
    payload: &[u8],
) -> Result<(), NetError> {
    if payload.len() > UDP_MAX_PAYLOAD {
        return Err(NetError::BufferTooSmall);
    }

    let udp_len = UDP_HEADER_LEN + payload.len();
    let mut segment = [0u8; MAX_FRAME_BYTES];

    segment[0..2].copy_from_slice(&src_port.to_be_bytes());
    segment[2..4].copy_from_slice(&dst_port.to_be_bytes());
    segment[4..6].copy_from_slice(&(udp_len as u16).to_be_bytes());
    segment[6..8].copy_from_slice(&0u16.to_be_bytes());
    segment[8..8 + payload.len()].copy_from_slice(payload);

    send_ipv4_packet_nonblocking(dst_ip, IP_PROTO_UDP, &segment[..udp_len])
}

fn send_tcp_segment(index: usize, flags: u16, payload: &[u8]) -> Result<(), NetError> {
    let (src_ip, dst_ip, src_port, dst_port, seq, ack) = unsafe {
        let Some(state) = STATE.as_ref() else {
            return Err(NetError::NotInitialized);
        };
        if index >= state.tcp_connections.len() {
            return Err(NetError::InvalidAddress);
        }
        let conn = state.tcp_connections[index];
        if !conn.used {
            return Err(NetError::InvalidAddress);
        }
        (
            state.local_ip,
            conn.remote_ip,
            conn.local_port,
            conn.remote_port,
            conn.seq,
            conn.ack,
        )
    };

    if payload.len() > (MAX_FRAME_BYTES - ETHERNET_HEADER_LEN - IPV4_HEADER_LEN - TCP_HEADER_LEN) {
        return Err(NetError::BufferTooSmall);
    }

    let tcp_len = TCP_HEADER_LEN + payload.len();
    let mut segment = [0u8; MAX_FRAME_BYTES];

    segment[0..2].copy_from_slice(&src_port.to_be_bytes());
    segment[2..4].copy_from_slice(&dst_port.to_be_bytes());
    segment[4..8].copy_from_slice(&seq.to_be_bytes());
    segment[8..12].copy_from_slice(&ack.to_be_bytes());
    segment[12] = (5u8) << 4;
    segment[13] = (flags & 0xFF) as u8;
    segment[14..16].copy_from_slice(&0x4000u16.to_be_bytes());
    segment[16..18].copy_from_slice(&0u16.to_be_bytes());
    segment[18..20].copy_from_slice(&0u16.to_be_bytes());
    segment[TCP_HEADER_LEN..TCP_HEADER_LEN + payload.len()].copy_from_slice(payload);

    let checksum = tcp_checksum(src_ip, dst_ip, &segment[..tcp_len]);
    segment[16..18].copy_from_slice(&checksum.to_be_bytes());

    send_ipv4_packet(dst_ip, IP_PROTO_TCP, &segment[..tcp_len])?;

    unsafe {
        if let Some(state) = STATE.as_mut() {
            if index < state.tcp_connections.len() {
                let conn = &mut state.tcp_connections[index];
                if conn.used {
                    let mut advance = payload.len() as u32;
                    if (flags & TCP_FLAG_SYN) != 0 {
                        advance = advance.wrapping_add(1);
                    }
                    if (flags & TCP_FLAG_FIN) != 0 {
                        advance = advance.wrapping_add(1);
                    }
                    conn.seq = conn.seq.wrapping_add(advance);
                    conn.last_activity_tick = timer::ticks();
                }
            }
        }
    }

    Ok(())
}

fn send_ipv4_packet(dst_ip: Ipv4Addr, protocol: u8, payload: &[u8]) -> Result<(), NetError> {
    let (src_ip, next_hop, identification) = unsafe {
        let Some(state) = STATE.as_mut() else {
            return Err(NetError::NotInitialized);
        };

        let next_hop = if same_subnet(dst_ip, state.local_ip, state.netmask) {
            dst_ip
        } else {
            state.gateway
        };

        let id = state.next_ip_id;
        state.next_ip_id = state.next_ip_id.wrapping_add(1).max(1);
        (state.local_ip, next_hop, id)
    };

    let dst_mac = resolve_arp_blocking(next_hop, ARP_RESOLVE_TIMEOUT_TICKS)?;

    let total_len = IPV4_HEADER_LEN + payload.len();
    if total_len > (MAX_FRAME_BYTES - ETHERNET_HEADER_LEN) {
        return Err(NetError::BufferTooSmall);
    }

    let mut ip_packet = [0u8; MAX_FRAME_BYTES];
    ip_packet[0] = 0x45;
    ip_packet[1] = 0;
    ip_packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    ip_packet[4..6].copy_from_slice(&identification.to_be_bytes());
    ip_packet[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip_packet[8] = 64;
    ip_packet[9] = protocol;
    ip_packet[10..12].copy_from_slice(&0u16.to_be_bytes());
    ip_packet[12..16].copy_from_slice(&src_ip.octets());
    ip_packet[16..20].copy_from_slice(&dst_ip.octets());
    ip_packet[20..20 + payload.len()].copy_from_slice(payload);

    let checksum = ipv4_checksum(&ip_packet[..IPV4_HEADER_LEN]);
    ip_packet[10..12].copy_from_slice(&checksum.to_be_bytes());

    send_ethernet_frame(dst_mac, ETH_TYPE_IPV4, &ip_packet[..total_len])
}

fn send_ipv4_packet_nonblocking(dst_ip: Ipv4Addr, protocol: u8, payload: &[u8]) -> Result<(), NetError> {
    let (src_ip, next_hop, identification) = unsafe {
        let Some(state) = STATE.as_mut() else {
            return Err(NetError::NotInitialized);
        };

        let next_hop = if same_subnet(dst_ip, state.local_ip, state.netmask) {
            dst_ip
        } else {
            state.gateway
        };

        let id = state.next_ip_id;
        state.next_ip_id = state.next_ip_id.wrapping_add(1).max(1);
        (state.local_ip, next_hop, id)
    };

    let now = timer::ticks();
    let Some(dst_mac) = arp_cache_lookup(next_hop, now) else {
        let _ = send_arp_request(next_hop);
        return Err(NetError::Busy);
    };
    unsafe {
        STATS.arp_hits = STATS.arp_hits.wrapping_add(1);
    }

    let total_len = IPV4_HEADER_LEN + payload.len();
    if total_len > (MAX_FRAME_BYTES - ETHERNET_HEADER_LEN) {
        return Err(NetError::BufferTooSmall);
    }

    let mut ip_packet = [0u8; MAX_FRAME_BYTES];
    ip_packet[0] = 0x45;
    ip_packet[1] = 0;
    ip_packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    ip_packet[4..6].copy_from_slice(&identification.to_be_bytes());
    ip_packet[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip_packet[8] = 64;
    ip_packet[9] = protocol;
    ip_packet[10..12].copy_from_slice(&0u16.to_be_bytes());
    ip_packet[12..16].copy_from_slice(&src_ip.octets());
    ip_packet[16..20].copy_from_slice(&dst_ip.octets());
    ip_packet[20..20 + payload.len()].copy_from_slice(payload);

    let checksum = ipv4_checksum(&ip_packet[..IPV4_HEADER_LEN]);
    ip_packet[10..12].copy_from_slice(&checksum.to_be_bytes());

    send_ethernet_frame(dst_mac, ETH_TYPE_IPV4, &ip_packet[..total_len])
}

fn send_ethernet_frame(dst_mac: [u8; 6], eth_type: u16, payload: &[u8]) -> Result<(), NetError> {
    let src_mac = unsafe {
        let Some(state) = STATE.as_ref() else {
            return Err(NetError::NotInitialized);
        };
        state.local_mac
    };

    let mut frame = [0u8; MAX_FRAME_BYTES];
    let mut len = ETHERNET_HEADER_LEN + payload.len();
    if len > frame.len() {
        return Err(NetError::BufferTooSmall);
    }

    frame[0..6].copy_from_slice(&dst_mac);
    frame[6..12].copy_from_slice(&src_mac);
    frame[12..14].copy_from_slice(&eth_type.to_be_bytes());
    frame[14..14 + payload.len()].copy_from_slice(payload);

    if len < 60 {
        frame[len..60].fill(0);
        len = 60;
    }

    ne2k::send_frame(&frame[..len])?;
    unsafe {
        STATS.tx_packets = STATS.tx_packets.wrapping_add(1);
    }
    Ok(())
}

fn resolve_arp_blocking(target_ip: Ipv4Addr, timeout_ticks: u32) -> Result<[u8; 6], NetError> {
    if let Some(mac) = arp_cache_lookup(target_ip, timer::ticks()) {
        unsafe {
            STATS.arp_hits = STATS.arp_hits.wrapping_add(1);
        }
        return Ok(mac);
    }

    let mut last_request_tick = 0u32;
    let start = timer::ticks();

    loop {
        let now = timer::ticks();
        if now.wrapping_sub(last_request_tick) > 20 {
            let _ = send_arp_request(target_ip);
            last_request_tick = now;
        }

        poll(now);

        if let Some(mac) = arp_cache_lookup(target_ip, now) {
            unsafe {
                STATS.arp_hits = STATS.arp_hits.wrapping_add(1);
            }
            return Ok(mac);
        }

        if now.wrapping_sub(start) > timeout_ticks {
            return Err(NetError::Timeout);
        }

        task::sleep_ticks(1);
    }
}

fn arp_cache_lookup(ip: Ipv4Addr, now_ticks: u32) -> Option<[u8; 6]> {
    let expiration = 30_000u32;

    unsafe {
        let state = STATE.as_ref()?;
        for entry in state.arp_cache.iter() {
            if !entry.valid {
                continue;
            }
            if entry.ip == ip && now_ticks.wrapping_sub(entry.last_seen_tick) <= expiration {
                return Some(entry.mac);
            }
        }
    }

    None
}

fn arp_cache_insert(ip: Ipv4Addr, mac: [u8; 6], now_ticks: u32) {
    unsafe {
        let Some(state) = STATE.as_mut() else {
            return;
        };

        for entry in state.arp_cache.iter_mut() {
            if entry.valid && entry.ip == ip {
                entry.mac = mac;
                entry.last_seen_tick = now_ticks;
                return;
            }
        }

        if let Some(slot) = state.arp_cache.iter_mut().find(|entry| !entry.valid) {
            *slot = ArpEntry {
                valid: true,
                ip,
                mac,
                last_seen_tick: now_ticks,
            };
            return;
        }

        let mut oldest = 0usize;
        for index in 1..state.arp_cache.len() {
            if state.arp_cache[index].last_seen_tick < state.arp_cache[oldest].last_seen_tick {
                oldest = index;
            }
        }

        state.arp_cache[oldest] = ArpEntry {
            valid: true,
            ip,
            mac,
            last_seen_tick: now_ticks,
        };
    }
}

fn parse_dns_response(expected_id: u16, payload: &[u8]) -> Option<Ipv4Addr> {
    if payload.len() < 12 {
        return None;
    }

    let id = u16::from_be_bytes([payload[0], payload[1]]);
    if id != expected_id {
        return None;
    }

    let flags = u16::from_be_bytes([payload[2], payload[3]]);
    if (flags & 0x8000) == 0 {
        return None;
    }
    if (flags & 0x000F) != 0 {
        return None;
    }

    let questions = u16::from_be_bytes([payload[4], payload[5]]) as usize;
    let answers = u16::from_be_bytes([payload[6], payload[7]]) as usize;

    let mut offset = 12usize;
    for _ in 0..questions {
        offset = skip_dns_name(payload, offset)?;
        offset = offset.checked_add(4)?;
        if offset > payload.len() {
            return None;
        }
    }

    for _ in 0..answers {
        offset = skip_dns_name(payload, offset)?;
        if offset + 10 > payload.len() {
            return None;
        }

        let answer_type = u16::from_be_bytes([payload[offset], payload[offset + 1]]);
        let answer_class = u16::from_be_bytes([payload[offset + 2], payload[offset + 3]]);
        let rdlen = u16::from_be_bytes([payload[offset + 8], payload[offset + 9]]) as usize;
        offset += 10;

        if offset + rdlen > payload.len() {
            return None;
        }

        if answer_type == 1 && answer_class == 1 && rdlen == 4 {
            return Some(Ipv4Addr::new(
                payload[offset],
                payload[offset + 1],
                payload[offset + 2],
                payload[offset + 3],
            ));
        }

        offset += rdlen;
    }

    None
}

fn build_dns_query_packet(id: u16, host: &str, out: &mut [u8; 512]) -> Result<usize, NetError> {
    out.fill(0);

    out[0..2].copy_from_slice(&id.to_be_bytes());
    out[2..4].copy_from_slice(&0x0100u16.to_be_bytes());
    out[4..6].copy_from_slice(&1u16.to_be_bytes());

    let mut offset = 12usize;
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(NetError::InvalidAddress);
        }

        if offset + 1 + label.len() >= out.len() {
            return Err(NetError::BufferTooSmall);
        }

        out[offset] = label.len() as u8;
        offset += 1;
        out[offset..offset + label.len()].copy_from_slice(label.as_bytes());
        offset += label.len();
    }

    if offset + 5 >= out.len() {
        return Err(NetError::BufferTooSmall);
    }

    out[offset] = 0;
    offset += 1;
    out[offset..offset + 2].copy_from_slice(&1u16.to_be_bytes());
    offset += 2;
    out[offset..offset + 2].copy_from_slice(&1u16.to_be_bytes());
    offset += 2;

    Ok(offset)
}

fn skip_dns_name(payload: &[u8], mut offset: usize) -> Option<usize> {
    let mut jumps = 0usize;

    loop {
        if offset >= payload.len() {
            return None;
        }

        let len = payload[offset];
        if (len & 0xC0) == 0xC0 {
            if offset + 1 >= payload.len() {
                return None;
            }
            offset += 2;
            return Some(offset);
        }

        offset += 1;
        if len == 0 {
            return Some(offset);
        }

        offset = offset.checked_add(len as usize)?;
        if offset > payload.len() {
            return None;
        }

        jumps += 1;
        if jumps > 128 {
            return None;
        }
    }
}

fn allocate_ephemeral_port(state: &mut NetStackState) -> u16 {
    let mut candidate = state.next_ephemeral_port;
    if candidate < 49152 {
        candidate = 49152;
    }

    state.next_ephemeral_port = state.next_ephemeral_port.wrapping_add(1);
    if state.next_ephemeral_port < 49152 {
        state.next_ephemeral_port = 49152;
    }

    candidate
}

fn same_subnet(a: Ipv4Addr, b: Ipv4Addr, mask: Ipv4Addr) -> bool {
    let ao = a.octets();
    let bo = b.octets();
    let mo = mask.octets();

    (ao[0] & mo[0]) == (bo[0] & mo[0])
        && (ao[1] & mo[1]) == (bo[1] & mo[1])
        && (ao[2] & mo[2]) == (bo[2] & mo[2])
        && (ao[3] & mo[3]) == (bo[3] & mo[3])
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    checksum16(header)
}

fn tcp_checksum(src_ip: Ipv4Addr, dst_ip: Ipv4Addr, segment: &[u8]) -> u16 {
    let mut sum = 0u32;

    let src = src_ip.octets();
    let dst = dst_ip.octets();

    sum = sum.wrapping_add(u16::from_be_bytes([src[0], src[1]]) as u32);
    sum = sum.wrapping_add(u16::from_be_bytes([src[2], src[3]]) as u32);
    sum = sum.wrapping_add(u16::from_be_bytes([dst[0], dst[1]]) as u32);
    sum = sum.wrapping_add(u16::from_be_bytes([dst[2], dst[3]]) as u32);
    sum = sum.wrapping_add(IP_PROTO_TCP as u32);
    sum = sum.wrapping_add(segment.len() as u32);

    let mut index = 0usize;
    while index + 1 < segment.len() {
        sum = sum.wrapping_add(u16::from_be_bytes([segment[index], segment[index + 1]]) as u32);
        index += 2;
    }

    if index < segment.len() {
        sum = sum.wrapping_add((segment[index] as u32) << 8);
    }

    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF).wrapping_add(sum >> 16);
    }

    !(sum as u16)
}

fn checksum16(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        sum = sum.wrapping_add(u16::from_be_bytes([bytes[index], bytes[index + 1]]) as u32);
        index += 2;
    }

    if index < bytes.len() {
        sum = sum.wrapping_add((bytes[index] as u32) << 8);
    }

    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF).wrapping_add(sum >> 16);
    }

    !(sum as u16)
}

fn parse_ipv4(input: &str) -> Option<Ipv4Addr> {
    let mut out = [0u8; 4];
    let mut index = 0usize;

    for part in input.split('.') {
        if index >= out.len() || part.is_empty() {
            return None;
        }

        let mut value = 0u16;
        for byte in part.bytes() {
            if !byte.is_ascii_digit() {
                return None;
            }
            value = value.checked_mul(10)?;
            value = value.checked_add((byte - b'0') as u16)?;
            if value > 255 {
                return None;
            }
        }

        out[index] = value as u8;
        index += 1;
    }

    if index != 4 {
        return None;
    }

    Some(Ipv4Addr { octets: out })
}

fn copy_ascii(dst: &mut [u8], src: &[u8]) -> usize {
    let mut written = 0usize;
    for byte in src.iter().copied() {
        if written >= dst.len() {
            break;
        }
        dst[written] = match byte {
            0x20..=0x7E => byte,
            _ => b'?',
        };
        written += 1;
    }
    written
}

fn push_u8_decimal(out: &mut [u8], start: usize, value: u8) -> usize {
    let mut digits = [0u8; 3];
    let mut v = value;
    let mut count = 0usize;

    loop {
        digits[count] = b'0' + (v % 10);
        count += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }

    let mut written = 0usize;
    for idx in (0..count).rev() {
        let target = start + written;
        if target >= out.len() {
            break;
        }
        out[target] = digits[idx];
        written += 1;
    }

    written
}
