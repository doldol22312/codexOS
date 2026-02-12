use crate::io;

const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;
const MAX_PCI_DEVICES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision_id: u8,
    pub header_type: u8,
    pub irq_line: u8,
    pub bar0: u32,
}

impl PciDevice {
    const fn empty() -> Self {
        Self {
            bus: 0,
            slot: 0,
            function: 0,
            vendor_id: 0,
            device_id: 0,
            class_code: 0,
            subclass: 0,
            prog_if: 0,
            revision_id: 0,
            header_type: 0,
            irq_line: 0,
            bar0: 0,
        }
    }

    pub fn is_multifunction(&self) -> bool {
        (self.header_type & 0x80) != 0
    }

    pub fn bar0_io_base(&self) -> Option<u16> {
        if (self.bar0 & 0x1) == 0 {
            return None;
        }
        Some((self.bar0 & 0xFFFC) as u16)
    }
}

static mut DEVICES: [PciDevice; MAX_PCI_DEVICES] = [PciDevice::empty(); MAX_PCI_DEVICES];
static mut DEVICE_COUNT: usize = 0;
static mut SCANNED: bool = false;

pub fn init() {
    let _ = scan();
}

pub fn scan() -> usize {
    unsafe {
        DEVICE_COUNT = 0;

        for bus in 0u16..=255 {
            for slot in 0u8..32 {
                let vendor = config_read_u16(bus as u8, slot, 0, 0x00);
                if vendor == 0xFFFF {
                    continue;
                }

                let header_type = config_read_u8(bus as u8, slot, 0, 0x0E);
                let function_count = if (header_type & 0x80) != 0 { 8 } else { 1 };

                for function in 0u8..function_count {
                    let vendor_id = config_read_u16(bus as u8, slot, function, 0x00);
                    if vendor_id == 0xFFFF {
                        continue;
                    }

                    if DEVICE_COUNT >= MAX_PCI_DEVICES {
                        SCANNED = true;
                        return DEVICE_COUNT;
                    }

                    let device_id = config_read_u16(bus as u8, slot, function, 0x02);
                    let class_code = config_read_u8(bus as u8, slot, function, 0x0B);
                    let subclass = config_read_u8(bus as u8, slot, function, 0x0A);
                    let prog_if = config_read_u8(bus as u8, slot, function, 0x09);
                    let revision_id = config_read_u8(bus as u8, slot, function, 0x08);
                    let header_type = config_read_u8(bus as u8, slot, function, 0x0E);
                    let irq_line = config_read_u8(bus as u8, slot, function, 0x3C);
                    let bar0 = config_read_u32(bus as u8, slot, function, 0x10);

                    DEVICES[DEVICE_COUNT] = PciDevice {
                        bus: bus as u8,
                        slot,
                        function,
                        vendor_id,
                        device_id,
                        class_code,
                        subclass,
                        prog_if,
                        revision_id,
                        header_type,
                        irq_line,
                        bar0,
                    };
                    DEVICE_COUNT += 1;
                }
            }
        }

        SCANNED = true;
        DEVICE_COUNT
    }
}

pub fn device_count() -> usize {
    ensure_scanned();
    unsafe { DEVICE_COUNT }
}

pub fn device(index: usize) -> Option<PciDevice> {
    ensure_scanned();
    unsafe {
        if index < DEVICE_COUNT {
            Some(DEVICES[index])
        } else {
            None
        }
    }
}

pub fn find_device(vendor_id: u16, device_id: u16) -> Option<PciDevice> {
    ensure_scanned();
    unsafe {
        for index in 0..DEVICE_COUNT {
            let device = DEVICES[index];
            if device.vendor_id == vendor_id && device.device_id == device_id {
                return Some(device);
            }
        }
    }
    None
}

pub fn find_class(class_code: u8, subclass: u8) -> Option<PciDevice> {
    ensure_scanned();
    unsafe {
        for index in 0..DEVICE_COUNT {
            let device = DEVICES[index];
            if device.class_code == class_code && device.subclass == subclass {
                return Some(device);
            }
        }
    }
    None
}

pub fn class_name(class_code: u8, subclass: u8) -> &'static str {
    match (class_code, subclass) {
        (0x01, 0x01) => "IDE controller",
        (0x02, 0x00) => "Ethernet controller",
        (0x03, 0x00) => "VGA controller",
        (0x06, 0x00) => "Host bridge",
        (0x06, 0x01) => "ISA bridge",
        (0x0C, 0x03) => "USB controller",
        _ => "Other",
    }
}

fn ensure_scanned() {
    unsafe {
        if !SCANNED {
            let _ = scan();
        }
    }
}

#[inline]
fn config_read_u8(bus: u8, slot: u8, function: u8, offset: u8) -> u8 {
    let value = config_read_u32(bus, slot, function, offset);
    let shift = ((offset & 0x3) * 8) as u32;
    ((value >> shift) & 0xFF) as u8
}

#[inline]
fn config_read_u16(bus: u8, slot: u8, function: u8, offset: u8) -> u16 {
    let value = config_read_u32(bus, slot, function, offset);
    let shift = ((offset & 0x2) * 8) as u32;
    ((value >> shift) & 0xFFFF) as u16
}

#[inline]
fn config_read_u32(bus: u8, slot: u8, function: u8, offset: u8) -> u32 {
    let address = pci_config_address(bus, slot, function, offset);
    unsafe {
        io::outl(PCI_CONFIG_ADDRESS, address);
        io::inl(PCI_CONFIG_DATA)
    }
}

#[inline]
fn pci_config_address(bus: u8, slot: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((function as u32) << 8)
        | ((offset as u32) & 0xFC)
}
