use std::ffi::c_void;
use std::io;
use std::mem::zeroed;

use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::{IDirect3DDevice, IDirect3DSurface};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::core::{Interface, factory};

use super::config::CAPTURE_BUFFER_COUNT;

pub(super) struct WindowFrameSource {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    direct3d_device: IDirect3DDevice,
    frame_pool: Direct3D11CaptureFramePool,
    _session: GraphicsCaptureSession,
    size: SizeInt32,
}

impl WindowFrameSource {
    pub(super) fn start(hwnd: isize) -> io::Result<Self> {
        if !GraphicsCaptureSession::IsSupported().map_err(windows_error_to_io)? {
            return Err(io::Error::other(
                "Windows.Graphics.Capture is not supported on this system",
            ));
        }

        let (device, context, direct3d_device) = create_d3d_device()?;
        let item = create_capture_item_for_window(hwnd)?;
        let size = item.Size().map_err(windows_error_to_io)?;
        if size.Width <= 0 || size.Height <= 0 {
            return Err(io::Error::other("target window has an empty capture size"));
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
        session.StartCapture().map_err(windows_error_to_io)?;

        Ok(Self {
            device,
            context,
            direct3d_device,
            frame_pool,
            _session: session,
            size,
        })
    }

    pub(super) fn try_next_frame(&mut self) -> io::Result<Option<CapturedFrame>> {
        let frame = match self.frame_pool.TryGetNextFrame() {
            Ok(frame) => frame,
            Err(_) => return Ok(None),
        };

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
        let surface = frame.Surface().map_err(windows_error_to_io)?;
        read_surface_bgra(&self.device, &self.context, &surface, content_size).map(
            |(bgra, row_pitch)| {
                Some(CapturedFrame {
                    bgra,
                    row_pitch,
                    system_relative_time,
                    content_size: ContentSize {
                        width: content_size.Width as u32,
                        height: content_size.Height as u32,
                    },
                })
            },
        )
    }
}

pub(super) struct CapturedFrame {
    pub(super) bgra: Vec<u8>,
    pub(super) row_pitch: usize,
    pub(super) system_relative_time: i64,
    pub(super) content_size: ContentSize,
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

fn create_capture_item_for_window(hwnd: isize) -> io::Result<GraphicsCaptureItem> {
    let interop: IGraphicsCaptureItemInterop =
        factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(windows_error_to_io)?;
    // SAFETY: hwnd comes from the selected top-level AppWindow. Windows validates
    // the handle and returns an error if it cannot create a capture item.
    unsafe { interop.CreateForWindow(HWND(hwnd as *mut c_void)) }.map_err(windows_error_to_io)
}

fn read_surface_bgra(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    surface: &IDirect3DSurface,
    content_size: SizeInt32,
) -> io::Result<(Vec<u8>, usize)> {
    let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(windows_error_to_io)?;
    // SAFETY: The WinRT surface comes from WGC and exposes its backing DXGI
    // texture through IDirect3DDxgiInterfaceAccess.
    let texture: ID3D11Texture2D = unsafe { access.GetInterface() }.map_err(windows_error_to_io)?;

    // SAFETY: desc points to valid writable storage for the texture description.
    let mut desc: D3D11_TEXTURE2D_DESC = unsafe { zeroed() };
    // SAFETY: texture is valid and writes only to desc.
    unsafe { texture.GetDesc(&mut desc) };
    desc.Width = content_size.Width as u32;
    desc.Height = content_size.Height as u32;
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

    let mut staging = None;
    // SAFETY: desc describes a CPU-readable staging texture. The out pointer is
    // valid Option storage owned by this function.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) }
        .map_err(windows_error_to_io)?;
    let staging =
        staging.ok_or_else(|| io::Error::other("D3D11 staging texture was not created"))?;

    let src: ID3D11Resource = texture.cast().map_err(windows_error_to_io)?;
    let dst: ID3D11Resource = staging.cast().map_err(windows_error_to_io)?;
    // SAFETY: src and dst are D3D11 resources on the same device. dst is a
    // staging texture with matching dimensions and format.
    unsafe { context.CopyResource(&dst, &src) };

    // SAFETY: mapped points to writable storage for D3D11 to describe the
    // mapped staging texture.
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { zeroed() };
    // SAFETY: dst is a CPU-readable staging resource and is unmapped below
    // before returning.
    unsafe { context.Map(&dst, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
        .map_err(windows_error_to_io)?;

    let row_pitch = mapped.RowPitch as usize;
    let bytes_len = row_pitch
        .checked_mul(content_size.Height as usize)
        .ok_or_else(|| io::Error::other("captured frame byte length overflow"))?;
    // SAFETY: Map returned a valid pointer to at least row_pitch * height bytes
    // for subresource 0. The data is copied before Unmap.
    let bytes =
        unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, bytes_len) }.to_vec();
    // SAFETY: This balances the successful Map call for dst subresource 0.
    unsafe { context.Unmap(&dst, 0) };

    Ok((bytes, row_pitch))
}

fn windows_error_to_io(error: windows::core::Error) -> io::Error {
    io::Error::other(error.to_string())
}
