use crate::{
    io,
    pci::{self, PciDevice},
    timer,
};

use super::NetError;

const SUPPORTED_IDS: &[(u16, u16)] = &[
    (0x10EC, 0x8029), // Realtek RTL8029 (NE2000-compatible)
    (0x1050, 0x0940), // Winbond 89C940
    (0x8E2E, 0x3000), // KTI ET32P2
];

const RX_START_PAGE: u8 = 0x46;
const RX_STOP_PAGE: u8 = 0x80;
const TX_START_PAGE: u8 = 0x40;

const REG_COMMAND: u16 = 0x00;
const REG_PSTART: u16 = 0x01;
const REG_PSTOP: u16 = 0x02;
const REG_BNRY: u16 = 0x03;
const REG_TPSR: u16 = 0x04;
const REG_TBCR0: u16 = 0x05;
const REG_TBCR1: u16 = 0x06;
const REG_ISR: u16 = 0x07;
const REG_RSAR0: u16 = 0x08;
const REG_RSAR1: u16 = 0x09;
const REG_RBCR0: u16 = 0x0A;
const REG_RBCR1: u16 = 0x0B;
const REG_RCR: u16 = 0x0C;
const REG_TCR: u16 = 0x0D;
const REG_DCR: u16 = 0x0E;
const REG_IMR: u16 = 0x0F;
const REG_DATA: u16 = 0x10;
const REG_RESET: u16 = 0x1F;

const REG_PAR0: u16 = 0x01;
const REG_CURR: u16 = 0x07;
const REG_MAR0: u16 = 0x08;

const CMD_STOP: u8 = 0x01;
const CMD_START: u8 = 0x02;
const CMD_TXP: u8 = 0x04;
const CMD_RD_READ: u8 = 0x08;
const CMD_RD_WRITE: u8 = 0x10;
const CMD_RD_ABORT: u8 = 0x20;
const CMD_PAGE0: u8 = 0x00;
const CMD_PAGE1: u8 = 0x40;

const ISR_PRX: u8 = 0x01;
const ISR_PTX: u8 = 0x02;
const ISR_RXE: u8 = 0x04;
const ISR_TXE: u8 = 0x08;
const ISR_OVW: u8 = 0x10;
const ISR_RDC: u8 = 0x40;

const RCR_AB: u8 = 0x04;
const RCR_MONITOR: u8 = 0x20;
const TCR_NORMAL: u8 = 0x00;
const TCR_LOOPBACK: u8 = 0x02;

const IMR_RX: u8 = 0x01;
const IMR_TX: u8 = 0x02;
const IMR_RXE: u8 = 0x04;
const IMR_TXE: u8 = 0x08;
const IMR_OVW: u8 = 0x10;

const MAX_FRAME_BYTES: usize = 1536;
const TX_TIMEOUT_SPINS: usize = 50_000;
const RDC_TIMEOUT_SPINS: usize = 50_000;
const TX_TIMEOUT_TICKS: u32 = 2;
const RDC_TIMEOUT_TICKS: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ne2kDevice {
    pub pci: PciDevice,
    pub io_base: u16,
    pub irq_line: u8,
}

#[derive(Clone, Copy)]
struct Ne2kState {
    device: Ne2kDevice,
    mac: [u8; 6],
    rx_start: u8,
    rx_stop: u8,
    tx_start: u8,
    initialized: bool,
    last_poll_tick: u32,
}

static mut STATE: Option<Ne2kState> = None;

pub fn probe() -> Option<Ne2kDevice> {
    let count = pci::device_count();
    for index in 0..count {
        let Some(device) = pci::device(index) else {
            continue;
        };

        if !is_supported(device.vendor_id, device.device_id) {
            continue;
        }

        let io_base = device.bar0_io_base().unwrap_or(0);
        if io_base == 0 {
            continue;
        }

        return Some(Ne2kDevice {
            pci: device,
            io_base,
            irq_line: device.irq_line,
        });
    }

    None
}

pub fn init_pci(device: Ne2kDevice) -> Result<(), NetError> {
    if device.io_base == 0 {
        return Err(NetError::Unsupported);
    }

    let base = device.io_base;

    reset_card(base);

    write_reg(base, REG_COMMAND, CMD_STOP | CMD_PAGE0 | CMD_RD_ABORT);
    write_reg(base, REG_DCR, 0x49);
    write_reg(base, REG_RBCR0, 0);
    write_reg(base, REG_RBCR1, 0);
    write_reg(base, REG_RCR, RCR_MONITOR);
    write_reg(base, REG_TCR, TCR_LOOPBACK);
    write_reg(base, REG_PSTART, RX_START_PAGE);
    write_reg(base, REG_PSTOP, RX_STOP_PAGE);
    write_reg(base, REG_BNRY, RX_START_PAGE);
    write_reg(base, REG_ISR, 0xFF);
    write_reg(base, REG_IMR, 0x00);

    let mut mac = read_mac_page1(base);
    if mac.iter().all(|byte| *byte == 0x00) || mac.iter().all(|byte| *byte == 0xFF) {
        if let Some(prom_mac) = read_mac_prom(base) {
            mac = prom_mac;
        }
    }

    if mac.iter().all(|byte| *byte == 0x00) {
        return Err(NetError::Unsupported);
    }

    write_reg(base, REG_COMMAND, CMD_STOP | CMD_PAGE1 | CMD_RD_ABORT);
    for (index, byte) in mac.iter().copied().enumerate() {
        write_reg(base, REG_PAR0 + index as u16, byte);
    }
    write_reg(base, REG_CURR, RX_START_PAGE + 1);
    for index in 0..8 {
        write_reg(base, REG_MAR0 + index as u16, 0xFF);
    }

    write_reg(base, REG_COMMAND, CMD_START | CMD_PAGE0 | CMD_RD_ABORT);
    write_reg(base, REG_TCR, TCR_NORMAL);
    write_reg(base, REG_RCR, RCR_AB);
    write_reg(base, REG_ISR, 0xFF);
    write_reg(base, REG_IMR, IMR_RX | IMR_TX | IMR_RXE | IMR_TXE | IMR_OVW);

    unsafe {
        STATE = Some(Ne2kState {
            device,
            mac,
            rx_start: RX_START_PAGE,
            rx_stop: RX_STOP_PAGE,
            tx_start: TX_START_PAGE,
            initialized: true,
            last_poll_tick: 0,
        });
    }

    Ok(())
}

pub fn initialized() -> bool {
    unsafe { STATE.as_ref().is_some_and(|state| state.initialized) }
}

pub fn device() -> Option<Ne2kDevice> {
    unsafe { STATE.as_ref().map(|state| state.device) }
}

pub fn mac_address() -> Option<[u8; 6]> {
    unsafe { STATE.as_ref().map(|state| state.mac) }
}

pub fn poll(now_ticks: u32) {
    let Some(base) = io_base() else {
        return;
    };

    unsafe {
        if let Some(state) = STATE.as_mut() {
            state.last_poll_tick = now_ticks;
        }
    }

    let isr = read_reg(base, REG_ISR);
    if isr != 0 {
        write_reg(base, REG_ISR, isr);
    }
}

pub fn recv_frame(out: &mut [u8]) -> Option<usize> {
    if out.is_empty() {
        return None;
    }

    let (base, rx_start, rx_stop) = unsafe {
        let state = STATE.as_ref()?;
        (state.device.io_base, state.rx_start, state.rx_stop)
    };

    write_reg(base, REG_COMMAND, CMD_START | CMD_PAGE0 | CMD_RD_ABORT);

    let mut bnry = read_reg(base, REG_BNRY);
    if bnry < rx_start || bnry >= rx_stop {
        bnry = rx_start;
        write_reg(base, REG_BNRY, bnry);
    }

    let next_page = if bnry.wrapping_add(1) >= rx_stop {
        rx_start
    } else {
        bnry.wrapping_add(1)
    };

    let curr = read_curr(base);
    if next_page == curr {
        return None;
    }

    let mut header = [0u8; 4];
    if remote_dma_read(base, (next_page as u16) << 8, &mut header).is_err() {
        return None;
    }

    let reported_next = header[1];
    let frame_len = u16::from_le_bytes([header[2], header[3]]) as usize;
    if frame_len < 4 || frame_len > MAX_FRAME_BYTES + 4 {
        let cleanup = prev_ring_page(reported_next, rx_start, rx_stop);
        write_reg(base, REG_BNRY, cleanup);
        return None;
    }

    let payload_len = frame_len - 4;
    let copy_len = payload_len.min(out.len());
    if read_ring_bytes(base, next_page, 4, &mut out[..copy_len], rx_start, rx_stop).is_err() {
        let cleanup = prev_ring_page(reported_next, rx_start, rx_stop);
        write_reg(base, REG_BNRY, cleanup);
        return None;
    }

    let cleanup = prev_ring_page(reported_next, rx_start, rx_stop);
    write_reg(base, REG_BNRY, cleanup);
    write_reg(base, REG_ISR, ISR_PRX | ISR_RXE | ISR_OVW);

    Some(copy_len)
}

pub fn send_frame(frame: &[u8]) -> Result<(), NetError> {
    if frame.is_empty() {
        return Err(NetError::InvalidAddress);
    }

    let (base, tx_start) = unsafe {
        let state = STATE.as_ref().ok_or(NetError::NotInitialized)?;
        (state.device.io_base, state.tx_start)
    };

    let send_len = frame.len().max(60);
    if send_len > MAX_FRAME_BYTES {
        return Err(NetError::BufferTooSmall);
    }

    let mut local = [0u8; MAX_FRAME_BYTES];
    local[..frame.len()].copy_from_slice(frame);
    if send_len > frame.len() {
        local[frame.len()..send_len].fill(0);
    }

    remote_dma_write(base, (tx_start as u16) << 8, &local[..send_len])?;

    write_reg(base, REG_TPSR, tx_start);
    write_reg(base, REG_TBCR0, (send_len & 0xFF) as u8);
    write_reg(base, REG_TBCR1, ((send_len >> 8) & 0xFF) as u8);

    write_reg(base, REG_COMMAND, CMD_START | CMD_PAGE0 | CMD_RD_ABORT | CMD_TXP);

    let start_tick = timer::ticks();
    let mut spins = 0usize;
    while spins < TX_TIMEOUT_SPINS {
        let isr = read_reg(base, REG_ISR);
        if (isr & (ISR_PTX | ISR_TXE)) != 0 {
            write_reg(base, REG_ISR, ISR_PTX | ISR_TXE);
            if (isr & ISR_TXE) != 0 {
                return Err(NetError::Unsupported);
            }
            return Ok(());
        }
        if timer::ticks().wrapping_sub(start_tick) > TX_TIMEOUT_TICKS {
            break;
        }
        core::hint::spin_loop();
        spins += 1;
    }

    Err(NetError::Timeout)
}

fn io_base() -> Option<u16> {
    unsafe { STATE.as_ref().map(|state| state.device.io_base) }
}

fn read_curr(base: u16) -> u8 {
    write_reg(base, REG_COMMAND, CMD_START | CMD_PAGE1 | CMD_RD_ABORT);
    let current = read_reg(base, REG_CURR);
    write_reg(base, REG_COMMAND, CMD_START | CMD_PAGE0 | CMD_RD_ABORT);
    current
}

fn read_mac_page1(base: u16) -> [u8; 6] {
    write_reg(base, REG_COMMAND, CMD_STOP | CMD_PAGE1 | CMD_RD_ABORT);
    let mut mac = [0u8; 6];
    for (index, byte) in mac.iter_mut().enumerate() {
        *byte = read_reg(base, REG_PAR0 + index as u16);
    }
    write_reg(base, REG_COMMAND, CMD_STOP | CMD_PAGE0 | CMD_RD_ABORT);
    mac
}

fn read_mac_prom(base: u16) -> Option<[u8; 6]> {
    write_reg(base, REG_COMMAND, CMD_STOP | CMD_PAGE0 | CMD_RD_ABORT);
    write_reg(base, REG_DCR, 0x48);

    let mut prom = [0u8; 32];
    remote_dma_read(base, 0, &mut prom).ok()?;

    write_reg(base, REG_DCR, 0x49);

    let mut mac = [0u8; 6];
    if prom[0] == prom[1] && prom[2] == prom[3] {
        for index in 0..6 {
            mac[index] = prom[index * 2];
        }
    } else {
        mac.copy_from_slice(&prom[0..6]);
    }

    Some(mac)
}

fn read_ring_bytes(
    base: u16,
    page: u8,
    offset: u16,
    out: &mut [u8],
    rx_start: u8,
    rx_stop: u8,
) -> Result<(), NetError> {
    if out.is_empty() {
        return Ok(());
    }

    let ring_start = (rx_start as u16) << 8;
    let ring_end = (rx_stop as u16) << 8;
    let mut address = ((page as u16) << 8).wrapping_add(offset);

    if address >= ring_end {
        address = ring_start.wrapping_add(address - ring_end);
    }

    let mut written = 0usize;
    while written < out.len() {
        let remaining = out.len() - written;
        let chunk = (ring_end as usize)
            .saturating_sub(address as usize)
            .min(remaining);
        if chunk == 0 {
            address = ring_start;
            continue;
        }

        remote_dma_read(base, address, &mut out[written..written + chunk])?;
        written += chunk;
        address = ring_start;
    }

    Ok(())
}

fn prev_ring_page(page: u8, rx_start: u8, rx_stop: u8) -> u8 {
    if page <= rx_start {
        rx_stop.saturating_sub(1)
    } else {
        page.saturating_sub(1)
    }
}

fn remote_dma_read(base: u16, address: u16, out: &mut [u8]) -> Result<(), NetError> {
    if out.is_empty() {
        return Ok(());
    }

    write_reg(base, REG_RBCR0, (out.len() & 0xFF) as u8);
    write_reg(base, REG_RBCR1, ((out.len() >> 8) & 0xFF) as u8);
    write_reg(base, REG_RSAR0, (address & 0xFF) as u8);
    write_reg(base, REG_RSAR1, ((address >> 8) & 0xFF) as u8);
    write_reg(base, REG_COMMAND, CMD_START | CMD_PAGE0 | CMD_RD_READ);

    for byte in out.iter_mut() {
        *byte = unsafe { io::inb(base + REG_DATA) };
    }

    wait_for_rdc(base)?;
    write_reg(base, REG_ISR, ISR_RDC);
    Ok(())
}

fn remote_dma_write(base: u16, address: u16, data: &[u8]) -> Result<(), NetError> {
    if data.is_empty() {
        return Ok(());
    }

    write_reg(base, REG_RBCR0, (data.len() & 0xFF) as u8);
    write_reg(base, REG_RBCR1, ((data.len() >> 8) & 0xFF) as u8);
    write_reg(base, REG_RSAR0, (address & 0xFF) as u8);
    write_reg(base, REG_RSAR1, ((address >> 8) & 0xFF) as u8);
    write_reg(base, REG_COMMAND, CMD_START | CMD_PAGE0 | CMD_RD_WRITE);

    for byte in data.iter().copied() {
        unsafe {
            io::outb(base + REG_DATA, byte);
        }
    }

    wait_for_rdc(base)?;
    write_reg(base, REG_ISR, ISR_RDC);
    Ok(())
}

fn wait_for_rdc(base: u16) -> Result<(), NetError> {
    let start_tick = timer::ticks();
    let mut spins = 0usize;
    while spins < RDC_TIMEOUT_SPINS {
        if (read_reg(base, REG_ISR) & ISR_RDC) != 0 {
            return Ok(());
        }
        if timer::ticks().wrapping_sub(start_tick) > RDC_TIMEOUT_TICKS {
            break;
        }
        core::hint::spin_loop();
        spins += 1;
    }

    Err(NetError::Timeout)
}

fn reset_card(base: u16) {
    let latch = unsafe { io::inb(base + REG_RESET) };
    unsafe {
        io::outb(base + REG_RESET, latch);
    }
    write_reg(base, REG_ISR, 0xFF);
}

#[inline]
fn write_reg(base: u16, reg: u16, value: u8) {
    unsafe {
        io::outb(base + reg, value);
    }
}

#[inline]
fn read_reg(base: u16, reg: u16) -> u8 {
    unsafe { io::inb(base + reg) }
}

fn is_supported(vendor: u16, device: u16) -> bool {
    SUPPORTED_IDS
        .iter()
        .any(|(supported_vendor, supported_device)| {
            *supported_vendor == vendor && *supported_device == device
        })
}
