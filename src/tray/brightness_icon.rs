use std::io;
use std::mem;
use std::ptr;

use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, DIB_RGB_COLORS, DeleteObject,
    GetBitmapBits, GetDC, GetDIBits, GetObjectW, ReleaseDC,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetIconInfo, HICON, ICONINFO,
};

const OFF: (u8, u8, u8) = (128, 128, 128);
const ON: (u8, u8, u8) = (255, 212, 59);
const SOCKET: (u8, u8, u8) = (0, 0, 0);

/// An application-owned, colorized copy of the original application icon.
pub(crate) struct BrightnessIcon(HICON);

impl BrightnessIcon {
    pub(crate) fn create(source: HICON, brightness: Option<u32>) -> io::Result<Self> {
        let mut source_info = ICONINFO::default();
        // SAFETY: `source_info` is writable and `source` is a live resource icon.
        if unsafe { GetIconInfo(source, &mut source_info) } == 0 {
            return Err(io::Error::last_os_error());
        }

        let result = colorize(source_info, brightness.unwrap_or(0));
        // GetIconInfo returns caller-owned bitmap copies.
        // SAFETY: both handles are either null or exclusively owned bitmap copies.
        unsafe {
            DeleteObject(source_info.hbmColor);
            DeleteObject(source_info.hbmMask);
        }
        result.map(Self)
    }

    pub(crate) fn handle(&self) -> HICON {
        self.0
    }
}

impl Drop for BrightnessIcon {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns the icon created by CreateIconIndirect.
        unsafe { DestroyIcon(self.0) };
    }
}

fn colorize(source: ICONINFO, brightness: u32) -> io::Result<HICON> {
    if source.hbmColor.is_null() || source.hbmMask.is_null() {
        return Err(io::Error::other("resource icon has no color bitmap"));
    }

    let mut bitmap = BITMAP::default();
    // SAFETY: `bitmap` has the exact type and size requested for the live bitmap handle.
    if unsafe {
        GetObjectW(
            source.hbmColor,
            mem::size_of::<BITMAP>() as i32,
            (&raw mut bitmap).cast(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let width = usize::try_from(bitmap.bmWidth).map_err(io::Error::other)?;
    let height = usize::try_from(bitmap.bmHeight).map_err(io::Error::other)?;
    let mut pixels = vec![0_u32; width * height];
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bitmap.bmWidth,
            biHeight: -bitmap.bmHeight,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };
    // SAFETY: a null window requests a screen DC.
    let dc = unsafe { GetDC(ptr::null_mut()) };
    if dc.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the DC, bitmap, output buffer, and bitmap description are valid.
    let lines = unsafe {
        GetDIBits(
            dc,
            source.hbmColor,
            0,
            height as u32,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    // SAFETY: `dc` came from GetDC for the same null window.
    unsafe { ReleaseDC(ptr::null_mut(), dc) };
    if lines != height as i32 {
        return Err(io::Error::last_os_error());
    }

    let source_pixels = pixels.clone();
    let mut original = vec![0_u32; source_pixels.len()];
    for y in 3..height {
        let source_row = (y - 3) * width;
        let destination_row = y * width;
        original[destination_row..destination_row + width]
            .copy_from_slice(&source_pixels[source_row..source_row + width]);
    }
    pixels.clone_from(&original);
    let bulb_pixels: Vec<bool> = original.iter().copied().map(is_bulb_fill).collect();
    let (red, green, blue) = color(brightness);
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let y = index / width;
        if is_socket(original[index], y, height) {
            let alpha = ((original[index] >> 24) as u8).max(224);
            *pixel = bgra(SOCKET.0, SOCKET.1, SOCKET.2, alpha);
        } else if bulb_pixels[index] {
            let alpha = (original[index] >> 24) as u8;
            *pixel = bgra(red, green, blue, alpha);
        }
    }
    let active_segments = brightness.min(100) / 20;
    for y in 0..height {
        for x in 0..width {
            if indicator_segment(x, y, width, height)
                .is_some_and(|segment| segment <= active_segments)
            {
                pixels[y * width + x] = bgra(ON.0, ON.1, ON.2, 230);
            }
        }
    }

    // SAFETY: the buffer contains width * height 32-bit pixels.
    let color_bitmap = unsafe {
        CreateBitmap(
            bitmap.bmWidth,
            bitmap.bmHeight,
            1,
            32,
            pixels.as_ptr().cast(),
        )
    };
    if color_bitmap.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut mask_description = BITMAP::default();
    // SAFETY: the destination has the exact type requested for the live mask bitmap.
    if unsafe {
        GetObjectW(
            source.hbmMask,
            mem::size_of::<BITMAP>() as i32,
            (&raw mut mask_description).cast(),
        )
    } == 0
    {
        // SAFETY: `color_bitmap` is exclusively owned here.
        unsafe { DeleteObject(color_bitmap) };
        return Err(io::Error::last_os_error());
    }
    let mask_stride = mask_description.bmWidthBytes as usize;
    let mask_size = mask_stride * mask_description.bmHeight as usize;
    let mut mask_bits = vec![0_u8; mask_size];
    // SAFETY: the buffer has the exact byte size reported for the mask bitmap.
    if unsafe {
        GetBitmapBits(
            source.hbmMask,
            mask_size as i32,
            mask_bits.as_mut_ptr().cast(),
        )
    } != mask_size as i32
    {
        // SAFETY: `color_bitmap` is exclusively owned here.
        unsafe { DeleteObject(color_bitmap) };
        return Err(io::Error::last_os_error());
    }
    for y in 0..height {
        for x in 0..width {
            if indicator_segment(x, y, width, height)
                .is_some_and(|segment| segment <= active_segments)
            {
                mask_bits[y * mask_stride + x / 8] &= !(0x80 >> (x % 8));
            }
        }
    }
    // SAFETY: the buffer has the dimensions and one-bit layout reported above.
    let mask_bitmap = unsafe {
        CreateBitmap(
            mask_description.bmWidth,
            mask_description.bmHeight,
            1,
            1,
            mask_bits.as_ptr().cast(),
        )
    };
    if mask_bitmap.is_null() {
        // SAFETY: `color_bitmap` is exclusively owned here.
        unsafe { DeleteObject(color_bitmap) };
        return Err(io::Error::last_os_error());
    }
    let icon_info = ICONINFO {
        hbmColor: color_bitmap,
        hbmMask: mask_bitmap,
        ..source
    };
    // SAFETY: `icon_info` contains compatible, live color and mask bitmaps.
    let icon = unsafe { CreateIconIndirect(&icon_info) };
    // CreateIconIndirect copies the color bitmap.
    // SAFETY: `color_bitmap` is exclusively owned here.
    unsafe { DeleteObject(color_bitmap) };
    // SAFETY: `mask_bitmap` is exclusively owned here.
    unsafe { DeleteObject(mask_bitmap) };
    if icon.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(icon)
    }
}

fn is_bulb_fill(pixel: u32) -> bool {
    let alpha = pixel >> 24;
    let blue = pixel & 0xff;
    let green = pixel >> 8 & 0xff;
    let red = pixel >> 16 & 0xff;
    alpha != 0 && red > blue.saturating_add(20) && green > blue.saturating_add(10)
}

fn is_socket(pixel: u32, y: usize, height: usize) -> bool {
    y >= height * 11 / 16 && pixel & 0x00ff_ffff != 0
}

fn indicator_segment(x: usize, y: usize, width: usize, height: usize) -> Option<u32> {
    let x = x * 32 / width;
    let y = y * 32 / height;
    [
        (2..=4, 14..=16),
        (5..=7, 3..=5),
        (15..=17, 0..=2),
        (25..=27, 3..=5),
        (28..=30, 14..=16),
    ]
    .iter()
    .position(|(xs, ys)| xs.contains(&x) && ys.contains(&y))
    .map(|index| index as u32 + 1)
}

fn bgra(red: u8, green: u8, blue: u8, alpha: u8) -> u32 {
    let scale = |component: u8| u32::from(component) * u32::from(alpha) / 255;
    scale(blue) | (scale(green) << 8) | (scale(red) << 16) | (u32::from(alpha) << 24)
}

fn color(brightness: u32) -> (u8, u8, u8) {
    let brightness = brightness.min(100);
    let mix = |off: u8, on: u8| {
        ((u32::from(off) * (100 - brightness) + u32::from(on) * brightness + 50) / 100) as u8
    };
    (mix(OFF.0, ON.0), mix(OFF.1, ON.1), mix(OFF.2, ON.2))
}

#[cfg(test)]
mod tests {
    use super::{OFF, ON, color};

    #[test]
    fn interpolates_icon_color_across_brightness_range() {
        assert_eq!(color(0), OFF);
        assert_eq!(color(50), (192, 170, 94));
        assert_eq!(color(100), ON);
        assert_eq!(color(101), ON);
    }
}
