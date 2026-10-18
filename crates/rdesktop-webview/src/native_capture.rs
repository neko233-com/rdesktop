//! Native compositor frames for one registered window, including OS chrome and
//! owned dialogs. Never captures a desktop region or changes window visibility.
use crate::native_windows::{frame_bounds, Anchor, FrameBounds};
use image::ImageEncoder;
use std::time::{Duration, Instant};
use windows::{
    core::{Interface, HRESULT},
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::*,
            Dxgi::{Common::DXGI_FORMAT_B8G8R8A8_UNORM, IDXGIDevice},
        },
        System::WinRT::{
            Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
            Graphics::Capture::IGraphicsCaptureItemInterop,
            RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED,
        },
    },
};

const MAX_PIXELS: u64 = 16_777_216;
struct Runtime;
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}
struct Session {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
struct Frame(Direct3D11CaptureFrame);
impl Drop for Frame {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}
struct Mapping<'a> {
    context: &'a ID3D11DeviceContext,
    texture: &'a ID3D11Texture2D,
}
impl Drop for Mapping<'_> {
    fn drop(&mut self) {
        unsafe {
            self.context.Unmap(self.texture, 0);
        }
    }
}
fn invalid() -> windows::core::Error {
    windows::core::Error::from_hresult(HRESULT(0x80070057_u32 as i32))
}
fn valid_size(width: i32, height: i32) -> bool {
    width > 0
        && height > 0
        && width <= 8192
        && height <= 8192
        && (width as u64) * (height as u64) <= MAX_PIXELS
}

fn pixels(hwnd: isize) -> windows::core::Result<(u32, u32, Vec<u8>)> {
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    let _runtime = Runtime;
    if !GraphicsCaptureSession::IsSupported()? {
        return Err(windows::core::Error::from_hresult(HRESULT(
            0x80004001_u32 as i32,
        )));
    }
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(HWND(hwnd as _))? };
    let size = item.Size()?;
    if !valid_size(size.Width, size.Height) {
        return Err(invalid());
    }
    let (mut device, mut context) = (None, None);
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    let device = device.ok_or_else(invalid)?;
    let context = context.ok_or_else(invalid)?;
    let dxgi: IDXGIDevice = device.cast()?;
    let direct: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &direct,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        1,
        size,
    )?;
    let session = match pool.CreateCaptureSession(&item) {
        Ok(session) => session,
        Err(error) => {
            let _ = pool.Close();
            return Err(error);
        }
    };
    let capture = Session { session, pool };
    // Older Windows builds can lack this optional setting; OS defaults apply.
    let _ = capture.session.SetIsCursorCaptureEnabled(false);
    let _ = capture.session.SetIncludeSecondaryWindows(false);
    capture.session.StartCapture()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let frame = loop {
        match capture.pool.TryGetNextFrame() {
            Ok(frame) => break Frame(frame),
            Err(error) if error.code().is_ok() => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            return Err(windows::core::Error::from_hresult(HRESULT(
                0x800705b4_u32 as i32,
            )));
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    let content = frame.0.ContentSize()?;
    if !valid_size(content.Width, content.Height) {
        return Err(invalid());
    }
    let (width, height) = (content.Width as u32, content.Height as u32);
    let access: IDirect3DDxgiInterfaceAccess = frame.0.Surface()?.cast()?;
    let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe {
        texture.GetDesc(&mut desc);
    }
    if width > desc.Width || height > desc.Height || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
        return Err(invalid());
    }
    desc.Width = width;
    desc.Height = height;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    let mut staging = None;
    unsafe {
        device.CreateTexture2D(&desc, None, Some(&mut staging))?;
    }
    let staging = staging.ok_or_else(invalid)?;
    let bounds = D3D11_BOX {
        left: 0,
        top: 0,
        front: 0,
        right: width,
        bottom: height,
        back: 1,
    };
    unsafe {
        context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &texture, 0, Some(&bounds));
    }
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
    }
    let _mapping = Mapping {
        context: &context,
        texture: &staging,
    };
    if mapped.pData.is_null() || mapped.RowPitch < width * 4 {
        return Err(invalid());
    }
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for row in 0..height as usize {
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (mapped.pData as *const u8).add(row * mapped.RowPitch as usize),
                width as usize * 4,
            )
        };
        for pixel in bytes.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    Ok((width, height, rgba))
}

pub(crate) struct Capture {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub frame: Option<FrameBounds>,
}
pub(crate) fn capture(
    anchor: Anchor,
    revalidate: impl Fn() -> Result<(), String>,
) -> Result<Capture, String> {
    revalidate()?;
    unsafe {
        if windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(anchor.hwnd as _) == 0
            || windows_sys::Win32::UI::WindowsAndMessaging::IsIconic(anchor.hwnd as _) != 0
        {
            return Err("native compositor capture requires a visible, non-minimized window; hidden WebViews can use screenshot".into());
        }
    }
    let before = frame_bounds(anchor);
    let(width,height,rgba)=pixels(anchor.hwnd).map_err(|error|format!("Windows Graphics Capture failed (HRESULT 0x{:08X}); requires a supported graphics session and Windows 10 1903+",error.code().0 as u32))?;
    revalidate()?;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|_| "native PNG encoding failed")?;
    revalidate()?;
    let after = frame_bounds(anchor);
    // WindowRect includes invisible resize borders. Only expose a pixel origin
    // when the compositor bounds stayed stable and match the captured frame.
    let frame = after.filter(|bounds| {
        before == after && bounds.width as u32 == width && bounds.height as u32 == height
    });
    Ok(Capture {
        png,
        width,
        height,
        frame,
    })
}
