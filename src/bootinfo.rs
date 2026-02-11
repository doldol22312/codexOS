#[derive(Clone, Copy)]
pub struct BootVideoInfo {
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bpp: usize,
    pub framebuffer_phys: usize,
    pub font_phys: usize,
    pub font_bytes: usize,
    pub font_height: usize,
    pub flags: u32,
}

impl BootVideoInfo {
    pub fn vbe_active(&self) -> bool {
        (self.flags & 0x0000_0001) != 0
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BootVideoInfoRaw {
    magic: u32,
    flags: u32,
    mode: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    framebuffer_phys: u32,
    font_phys: u32,
    font_bytes: u32,
    font_height: u32,
}

const BOOT_VIDEO_INFO_PHYS: usize = 0x0000_5000;
const BOOT_VIDEO_MAGIC: u32 = 0x3245_4256;

pub fn video_info() -> Option<BootVideoInfo> {
    let raw = unsafe { (BOOT_VIDEO_INFO_PHYS as *const BootVideoInfoRaw).read_volatile() };
    if raw.magic != BOOT_VIDEO_MAGIC {
        return None;
    }

    Some(BootVideoInfo {
        width: raw.width as usize,
        height: raw.height as usize,
        pitch: raw.pitch as usize,
        bpp: raw.bpp as usize,
        framebuffer_phys: raw.framebuffer_phys as usize,
        font_phys: raw.font_phys as usize,
        font_bytes: raw.font_bytes as usize,
        font_height: raw.font_height as usize,
        flags: raw.flags,
    })
}
