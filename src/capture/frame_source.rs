use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};

use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::{IDirect3DDevice, IDirect3DSurface};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow, IsWindowVisible};
use windows::core::{Interface, factory};

use super::config::CAPTURE_BUFFER_COUNT;
use super::timing::CaptureTimeMapper;

pub(super) struct WindowFrameSource {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    direct3d_device: IDirect3DDevice,
    frame_pool: Direct3D11CaptureFramePool,
    _session: GraphicsCaptureSession,
    size: SizeInt32,
    hwnd: HWND,
    monitor: HMONITOR,
    staging: Option<ID3D11Texture2D>,
}

impl WindowFrameSource {
    pub(super) fn start(hwnd: isize) -> io::Result<(Self, i64)> {
        if !GraphicsCaptureSession::IsSupported().map_err(windows_error_to_io)? {
            return Err(io::Error::other(
                "Windows.Graphics.Capture is not supported on this system",
            ));
        }

        // Limit DPI awareness changes to this capture thread. Input hooks and
        // ScreenToClient in the mouse logger retain their existing context.
        let _dpi = PhysicalPixelContext::enter()?;
        let hwnd = HWND(hwnd as *mut c_void);
        // SAFETY: Windows validates the selected handle before monitor lookup.
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(io::Error::other("target window is no longer valid"));
        }
        // SAFETY: hwnd is a valid selected window; the monitor is fixed at start.
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let (device, context, direct3d_device) = create_d3d_device()?;
        let item = create_capture_item_for_monitor(monitor)?;
        let size = item.Size().map_err(windows_error_to_io)?;
        if size.Width <= 0 || size.Height <= 0 {
            return Err(io::Error::other("target monitor has an empty capture size"));
        }

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &direct3d_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            CAPTURE_BUFFER_COUNT,
            size,
        )
        .map_err(windows_error_to_io)?;
        let session = frame_pool
            .CreateCaptureSession(&item)
            .map_err(windows_error_to_io)?;
        let started_at_100ns = CaptureTimeMapper::now_100ns()?;
        session.StartCapture().map_err(windows_error_to_io)?;

        Ok((
            Self {
                device,
                context,
                direct3d_device,
                frame_pool,
                _session: session,
                size,
                hwnd,
                monitor,
                staging: None,
            },
            started_at_100ns,
        ))
    }

    pub(super) fn try_next_frame(&mut self) -> io::Result<Option<CapturedFrame>> {
        let mut raw_frame = std::ptr::null_mut();
        // SAFETY: Use the same ABI as windows-rs, but inspect successful null
        // separately. Its generated wrapper turns null into an error.
        // https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool.trygetnextframe
        unsafe {
            (Interface::vtable(&self.frame_pool).TryGetNextFrame)(
                Interface::as_raw(&self.frame_pool),
                &mut raw_frame,
            )
            .ok()
        }
        .map_err(windows_error_to_io)?;
        if raw_frame.is_null() {
            return Ok(None);
        }
        // SAFETY: A successful non-null ABI result transfers one owned reference.
        let frame = unsafe { Direct3D11CaptureFrame::from_raw(raw_frame) };

        let content_size = frame.ContentSize().map_err(windows_error_to_io)?;
        if content_size.Width <= 0 || content_size.Height <= 0 {
            return Ok(None);
        }

        if content_size != self.size {
            self.size = content_size;
            self.frame_pool
                .Recreate(
                    &self.direct3d_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    CAPTURE_BUFFER_COUNT,
                    self.size,
                )
                .map_err(windows_error_to_io)?;
            return Ok(None);
        }

        let system_relative_time = frame
            .SystemRelativeTime()
            .map_err(windows_error_to_io)?
            .Duration;
        // Snapshot geometry while acquiring/cropping this observation, never on
        // the encoder thread. It travels with the pixels through all padding.
        let _dpi = PhysicalPixelContext::enter()?;
        let Some(window) = visible_window_bounds(self.hwnd) else {
            return Ok(None);
        };
        let monitor = monitor_bounds(self.monitor)?;
        // During display mode changes the queued surface may have the old size.
        // Wait for matching geometry rather than interpreting it at a new scale.
        if i64::from(monitor.right) - i64::from(monitor.left) != i64::from(content_size.Width)
            || i64::from(monitor.bottom) - i64::from(monitor.top) != i64::from(content_size.Height)
        {
            return Ok(None);
        }
        let Some(crop) = CropRegion::intersect(window, monitor) else {
            return Ok(None);
        };
        let surface = frame.Surface().map_err(windows_error_to_io)?;
        read_surface_bgra(
            &self.device,
            &self.context,
            &surface,
            crop,
            &mut self.staging,
        )
        .map(|pixels| {
            pixels.map(|(bgra, row_pitch)| CapturedFrame {
                bgra,
                row_pitch,
                system_relative_time,
                content_size: ContentSize {
                    width: crop.width,
                    height: crop.height,
                },
                window_x: crop.window_x,
                window_y: crop.window_y,
            })
        })
    }
}

impl WindowFrameSource {
    pub(super) fn stop(&self) -> io::Result<()> {
        self._session.Close().map_err(windows_error_to_io)?;
        self.frame_pool.Close().map_err(windows_error_to_io)
    }
}

pub(super) struct CapturedFrame {
    pub(super) bgra: Vec<u8>,
    pub(super) row_pitch: usize,
    pub(super) system_relative_time: i64,
    pub(super) content_size: ContentSize,
    pub(super) window_x: i32,
    pub(super) window_y: i32,
}

/// Physical desktop origin and monitor-local GPU copy region (exclusive end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CropRegion {
    window_x: i32,
    window_y: i32,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

impl CropRegion {
    fn intersect(window: RECT, monitor: RECT) -> Option<Self> {
        if window.right <= window.left
            || window.bottom <= window.top
            || monitor.right <= monitor.left
            || monitor.bottom <= monitor.top
        {
            return None;
        }
        let left = window.left.max(monitor.left);
        let top = window.top.max(monitor.top);
        let right = window.right.min(monitor.right);
        let bottom = window.bottom.min(monitor.bottom);
        if right <= left || bottom <= top {
            return None;
        }
        Some(Self {
            window_x: left,
            window_y: top,
            left: u32::try_from(i64::from(left) - i64::from(monitor.left)).ok()?,
            top: u32::try_from(i64::from(top) - i64::from(monitor.top)).ok()?,
            width: u32::try_from(i64::from(right) - i64::from(left)).ok()?,
            height: u32::try_from(i64::from(bottom) - i64::from(top)).ok()?,
        })
    }

    fn fits_texture(self, width: u32, height: u32) -> bool {
        self.width > 0
            && self.height > 0
            && self
                .left
                .checked_add(self.width)
                .is_some_and(|right| right <= width)
            && self
                .top
                .checked_add(self.height)
                .is_some_and(|bottom| bottom <= height)
    }
}

fn visible_window_bounds(hwnd: HWND) -> Option<RECT> {
    // SAFETY: These APIs validate OS handles. Minimized/hidden/destroyed windows
    // supply no new observation, allowing the existing timeline to duplicate.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool()
            || IsIconic(hwnd).as_bool()
            || !IsWindowVisible(hwnd).as_bool()
        {
            return None;
        }
        let mut bounds = RECT::default();
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut bounds as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
        .ok()?;
        Some(bounds)
    }
}

fn monitor_bounds(monitor: HMONITOR) -> io::Result<RECT> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: info is correctly sized writable storage. Caller enters a
    // per-monitor-aware thread context so monitor and DWM bounds share pixels.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return Err(io::Error::last_os_error());
    }
    Ok(info.rcMonitor)
}

struct PhysicalPixelContext(DPI_AWARENESS_CONTEXT);

impl PhysicalPixelContext {
    fn enter() -> io::Result<Self> {
        // SAFETY: Changes only the calling thread; Drop restores its context.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.0.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(previous))
    }
}

impl Drop for PhysicalPixelContext {
    fn drop(&mut self) {
        // SAFETY: The saved context was returned on this same capture thread.
        unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}

#[derive(Clone, Copy)]
pub(super) struct ContentSize {
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(super) struct WinRtApartment;

impl WinRtApartment {
    pub(super) fn init() -> io::Result<Self> {
        // SAFETY: The capture worker owns this thread for WinRT work and pairs
        // successful initialization with RoUninitialize in Drop.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(windows_error_to_io)?;
        Ok(Self)
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        // SAFETY: This balances the successful RoInitialize call on the same thread.
        unsafe { RoUninitialize() };
    }
}

fn create_d3d_device() -> io::Result<(ID3D11Device, ID3D11DeviceContext, IDirect3DDevice)> {
    let mut device = None;
    let mut context = None;
    let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];

    // SAFETY: Output pointers are valid Option storage, the feature-level slice
    // remains alive for the call, and the hardware driver is requested without
    // a software module.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(windows_error_to_io)?;

    let device: ID3D11Device =
        device.ok_or_else(|| io::Error::other("D3D11 device was not created"))?;
    let context: ID3D11DeviceContext =
        context.ok_or_else(|| io::Error::other("D3D11 device context was not created"))?;
    let dxgi_device: IDXGIDevice = device.cast().map_err(windows_error_to_io)?;
    // SAFETY: dxgi_device is produced by the D3D11 device and remains valid for
    // the duration of this conversion to a WinRT Direct3D device.
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
        .map_err(windows_error_to_io)?;
    let direct3d_device = inspectable.cast().map_err(windows_error_to_io)?;

    Ok((device, context, direct3d_device))
}

fn create_capture_item_for_monitor(monitor: HMONITOR) -> io::Result<GraphicsCaptureItem> {
    let interop: IGraphicsCaptureItemInterop =
        factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(windows_error_to_io)?;
    // SAFETY: monitor comes from the selected window. Windows validates it.
    unsafe { interop.CreateForMonitor(monitor) }.map_err(windows_error_to_io)
}

fn read_surface_bgra(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    surface: &IDirect3DSurface,
    crop: CropRegion,
    staging: &mut Option<ID3D11Texture2D>,
) -> io::Result<Option<(Vec<u8>, usize)>> {
    let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(windows_error_to_io)?;
    // SAFETY: The WinRT surface comes from WGC and exposes its backing DXGI
    // texture through IDirect3DDxgiInterfaceAccess.
    let texture: ID3D11Texture2D = unsafe { access.GetInterface() }.map_err(windows_error_to_io)?;

    read_texture_bgra(device, context, &texture, crop, staging)
}

fn read_texture_bgra(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    crop: CropRegion,
    staging: &mut Option<ID3D11Texture2D>,
) -> io::Result<Option<(Vec<u8>, usize)>> {
    // SAFETY: desc points to valid writable storage for the texture description.
    let mut desc: D3D11_TEXTURE2D_DESC = unsafe { zeroed() };
    // SAFETY: texture is valid and writes only to desc.
    unsafe { texture.GetDesc(&mut desc) };
    // CopySubresourceRegion has undefined behavior for out-of-range boxes.
    if !crop.fits_texture(desc.Width, desc.Height) {
        return Ok(None);
    }
    desc.Width = crop.width;
    desc.Height = crop.height;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;
    desc.SampleDesc = DXGI_SAMPLE_DESC {
        Count: 1,
        Quality: 0,
    };
    desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;

    let reusable = staging.as_ref().is_some_and(|texture| {
        let mut old = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: texture is valid and old is writable storage.
        unsafe { texture.GetDesc(&mut old) };
        old.Width == desc.Width && old.Height == desc.Height
    });
    if !reusable {
        let mut new_staging = None;
        // SAFETY: desc describes a cropped CPU-readable staging texture.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut new_staging)) }
            .map_err(windows_error_to_io)?;
        *staging = Some(
            new_staging.ok_or_else(|| io::Error::other("D3D11 staging texture was not created"))?,
        );
    }
    let staging = staging.as_ref().expect("staging texture initialized above");

    let src: ID3D11Resource = texture.cast().map_err(windows_error_to_io)?;
    let dst: ID3D11Resource = staging.cast().map_err(windows_error_to_io)?;
    let source_box = D3D11_BOX {
        left: crop.left,
        top: crop.top,
        front: 0,
        right: crop.left + crop.width,
        bottom: crop.top + crop.height,
        back: 1,
    };
    // SAFETY: The validated box fits src, and dst matches the box size/format.
    // Only the crop is copied on the GPU and read back to the CPU.
    unsafe { context.CopySubresourceRegion(&dst, 0, 0, 0, 0, &src, 0, Some(&source_box)) };

    // SAFETY: mapped points to writable storage for D3D11 to describe the
    // mapped staging texture.
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { zeroed() };
    // SAFETY: dst is a CPU-readable staging resource and is unmapped below
    // before returning.
    unsafe { context.Map(&dst, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
        .map_err(windows_error_to_io)?;

    let row_pitch = mapped.RowPitch as usize;
    let result = (|| {
        let bytes_len = row_pitch
            .checked_mul(crop.height as usize)
            .filter(|length| *length <= isize::MAX as usize)
            .ok_or_else(|| io::Error::other("captured frame byte length overflow"))?;
        if mapped.pData.is_null() || row_pitch < crop.width as usize * 4 {
            return Err(io::Error::other("invalid mapped capture texture"));
        }
        // SAFETY: Map returned a valid pointer to at least row_pitch * height bytes
        // for subresource 0. The data is copied before Unmap.
        let bytes =
            unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, bytes_len) }.to_vec();
        Ok(Some((bytes, row_pitch)))
    })();
    // SAFETY: This balances the successful Map call for dst subresource 0.
    unsafe { context.Unmap(&dst, 0) };

    result
}

fn windows_error_to_io(error: windows::core::Error) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Direct3D11::{D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DEFAULT};
    use windows::Win32::UI::HiDpi::GetThreadDpiAwarenessContext;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn crop_uses_physical_pixels_on_negative_origin_monitor_at_100_and_125_percent() {
        for (width, height) in [(800, 600), (1000, 750)] {
            // Bounds supplied by DWM are already physical: no second DPI scale.
            let crop = CropRegion::intersect(
                rect(-1800, -900, -1800 + width, -900 + height),
                rect(-1920, -1080, 0, 0),
            )
            .unwrap();
            assert_eq!(
                crop,
                CropRegion {
                    window_x: -1800,
                    window_y: -900,
                    left: 120,
                    top: 180,
                    width: width as u32,
                    height: height as u32,
                }
            );
        }
    }

    #[test]
    fn crop_tracks_moves_resizes_and_clips_each_monitor_edge() {
        let monitor = rect(-1920, 0, 0, 1080);
        for (window, expected) in [
            (rect(-1800, 20, -1000, 620), (-1800, 20, 120, 20, 800, 600)),
            (rect(-1700, 50, -700, 800), (-1700, 50, 220, 50, 1000, 750)),
            (rect(-2000, -100, -1000, 500), (-1920, 0, 0, 0, 920, 500)),
            (rect(-500, 800, 100, 1200), (-500, 800, 1420, 800, 500, 280)),
            (rect(-2000, -100, 100, 1200), (-1920, 0, 0, 0, 1920, 1080)),
        ] {
            let crop = CropRegion::intersect(window, monitor).unwrap();
            assert_eq!(
                (
                    crop.window_x,
                    crop.window_y,
                    crop.left,
                    crop.top,
                    crop.width,
                    crop.height
                ),
                expected
            );
            assert!(crop.fits_texture(1920, 1080));
        }
    }

    #[test]
    fn empty_offscreen_inverted_and_extreme_regions_are_safe() {
        let monitor = rect(0, 0, 1920, 1080);
        for window in [
            rect(5, 5, 5, 6),
            rect(6, 5, 5, 6),
            rect(0, 6, 1, 5),
            rect(-100, 0, 0, 100),
            rect(1920, 0, 2000, 100),
            rect(0, 1080, 10, 1200),
        ] {
            assert!(CropRegion::intersect(window, monitor).is_none());
        }
        assert!(CropRegion::intersect(monitor, rect(0, 0, 0, 1)).is_none());
        let crop = CropRegion::intersect(
            rect(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
            rect(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
        )
        .unwrap();
        assert_eq!((crop.width, crop.height), (u32::MAX, u32::MAX));
        assert!(
            !CropRegion {
                left: u32::MAX,
                ..crop
            }
            .fits_texture(u32::MAX, u32::MAX)
        );
        assert!(!CropRegion { width: 0, ..crop }.fits_texture(u32::MAX, u32::MAX));
    }

    #[test]
    fn physical_pixel_context_restores_thread_awareness_and_invalid_window_is_skipped() {
        // SAFETY: Reads only the calling thread's DPI context.
        let before = unsafe { GetThreadDpiAwarenessContext() };
        {
            let _context = PhysicalPixelContext::enter().unwrap();
            assert!(visible_window_bounds(HWND::default()).is_none());
        }
        // SAFETY: Reads only the calling thread's DPI context.
        assert_eq!(unsafe { GetThreadDpiAwarenessContext() }, before);
    }

    #[test]
    fn gpu_crop_reads_only_selected_pixels_and_reuses_staging_until_resize() {
        let _winrt = WinRtApartment::init().unwrap();
        let (device, context, _) = create_d3d_device().unwrap();
        // Each pixel identifies its original location, including row orientation.
        let pixels: Vec<u8> = (0..4)
            .flat_map(|y| (0..6).flat_map(move |x| [x, y, 90, 255]))
            .collect();
        let desc = D3D11_TEXTURE2D_DESC {
            Width: 6,
            Height: 4,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            ..Default::default()
        };
        let initial = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const c_void,
            SysMemPitch: 6 * 4,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        // SAFETY: Initial data covers the entire texture and lives through creation.
        unsafe { device.CreateTexture2D(&desc, Some(&initial), Some(&mut texture)) }.unwrap();
        let texture = texture.unwrap();
        let mut staging = None;
        for window in [
            rect(-9, 21, -6, 23),
            rect(-8, 22, -5, 24),
            rect(-10, 20, -8, 21),
        ] {
            let crop = CropRegion::intersect(window, rect(-10, 20, -4, 24)).unwrap();
            let previous = staging.clone();
            let (bytes, pitch) = read_texture_bgra(&device, &context, &texture, crop, &mut staging)
                .unwrap()
                .unwrap();
            assert_eq!(bytes.len(), pitch * crop.height as usize);
            for y in 0..crop.height {
                for x in 0..crop.width {
                    let offset = y as usize * pitch + x as usize * 4;
                    assert_eq!(
                        &bytes[offset..offset + 4],
                        &[(crop.left + x) as u8, (crop.top + y) as u8, 90, 255]
                    );
                }
            }
            if let Some(previous) = previous {
                assert_eq!(previous == *staging.as_ref().unwrap(), crop.width == 3);
            }
        }
        let invalid = CropRegion {
            window_x: 0,
            window_y: 0,
            left: 5,
            top: 3,
            width: 2,
            height: 2,
        };
        assert!(
            read_texture_bgra(&device, &context, &texture, invalid, &mut staging)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    #[ignore = "requires an unlocked desktop; briefly displays two temporary test windows"]
    fn monitor_capture_includes_popup_and_tracks_window_geometry() {
        use std::time::{Duration, Instant};
        use windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTOPRIMARY;
        use windows::Win32::UI::WindowsAndMessaging::*;
        use windows::core::w;

        struct TestWindow(HWND);
        impl Drop for TestWindow {
            fn drop(&mut self) {
                // SAFETY: Test windows are created and destroyed on this thread.
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        fn pump() {
            let mut message = MSG::default();
            // SAFETY: Pumps only messages belonging to this test's thread.
            unsafe {
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        let _winrt = WinRtApartment::init().unwrap();
        let _dpi = PhysicalPixelContext::enter().unwrap();
        // SAFETY: The desktop handle is used only to locate the primary monitor.
        let monitor = unsafe { MonitorFromWindow(GetDesktopWindow(), MONITOR_DEFAULTTOPRIMARY) };
        let bounds = monitor_bounds(monitor).unwrap();
        let (x, y) = (bounds.left + 100, bounds.top + 100);
        // STATIC styles SS_BLACKRECT (4) and SS_WHITERECT (6) provide predictable
        // content without depending on Office, custom window classes or GDI paint.
        // SAFETY: Built-in class, no borrowed callback data, and live test thread.
        let main = TestWindow(
            unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                    w!("STATIC"),
                    w!("Capture test"),
                    WS_POPUP | WS_VISIBLE | WINDOW_STYLE(4),
                    x,
                    y,
                    320,
                    240,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap(),
        );
        // SAFETY: Independent top-level popup, deliberately not a child of main.
        let popup = TestWindow(
            unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                    w!("STATIC"),
                    w!("Popup test"),
                    WS_POPUP | WS_VISIBLE | WINDOW_STYLE(6),
                    x + 20,
                    y + 20,
                    64,
                    64,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap(),
        );
        pump();
        // Like production, acquire/close WGC on a worker while the window owner
        // continues dispatching messages (Close can wait on the owning UI).
        let target_hwnd = main.0.0 as isize;
        let (frames_tx, frames_rx) = std::sync::mpsc::sync_channel(2);
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let capture = std::thread::spawn(move || -> io::Result<()> {
            let _winrt = WinRtApartment::init()?;
            let (mut source, _) = WindowFrameSource::start(target_hwnd)?;
            while matches!(
                stop_rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ) {
                if let Some(frame) = source.try_next_frame()? {
                    let _ = frames_tx.try_send(frame);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            source.stop()
        });
        for (dx, dy, width, height) in [(0, 0, 320, 240), (60, 40, 400, 300)] {
            // SAFETY: All handles are live; no activation is requested.
            unsafe {
                SetWindowPos(
                    main.0,
                    Some(HWND_TOPMOST),
                    x + dx,
                    y + dy,
                    width,
                    height,
                    SWP_NOACTIVATE,
                )
                .unwrap();
                SetWindowPos(
                    popup.0,
                    Some(HWND_TOPMOST),
                    x + dx + 20,
                    y + dy + 20,
                    64,
                    64,
                    SWP_NOACTIVATE,
                )
                .unwrap();
            }
            pump();
            let expected = visible_window_bounds(main.0).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut observations = 0;
            let mut sampled_colors = None;
            loop {
                pump();
                if let Ok(frame) = frames_rx.try_recv() {
                    observations += 1;
                    // Observations queued before a move retain their old bounds.
                    if (frame.window_x, frame.window_y) != (expected.left, expected.top)
                        || (frame.content_size.width, frame.content_size.height)
                            != (width as u32, height as u32)
                    {
                        continue;
                    }
                    let white = 30 * frame.row_pitch + 30 * 4;
                    let black = 100 * frame.row_pitch + 100 * 4;
                    sampled_colors = Some((
                        frame.bgra[white..white + 4].to_vec(),
                        frame.bgra[black..black + 4].to_vec(),
                    ));
                    if frame.bgra[white..white + 3].iter().all(|v| *v > 240)
                        // Themes may draw SS_BLACKRECT in gray. It must remain
                        // darker than the separate white popup in the crop.
                        && frame.bgra[black..black + 3].iter().all(|v| *v < 240)
                    {
                        break;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "monitor crop did not contain the overlaid popup: {observations} frames, colors {sampled_colors:?}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        // SAFETY: Hide a live test window; invalid geometry must not be read.
        let _ = unsafe { ShowWindow(main.0, SW_HIDE) };
        assert!(visible_window_bounds(main.0).is_none());
        stop_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !capture.is_finished() {
            pump();
            assert!(Instant::now() < deadline, "capture worker did not stop");
            std::thread::sleep(Duration::from_millis(5));
        }
        capture.join().unwrap().unwrap();
    }
}
