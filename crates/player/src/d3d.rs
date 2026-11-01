//! D3D11 device for mpv, a private render target, and keyed-mutex shared textures that the
//! patched GPUI renderer opens (see docs/player/video-surface.md).

use std::ffi::c_void;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
  D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
  D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
  D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX, D3D11_SDK_VERSION,
  D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
  ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
  CreateDXGIFactory1, IDXGIFactory1, IDXGIKeyedMutex, IDXGIResource,
};
use windows::core::Interface;

/// The D3D11 device mpv renders and decodes with.
pub struct VideoDevice {
  device: ID3D11Device,
  context: ID3D11DeviceContext,
}

// SAFETY: ID3D11Device is free-threaded; the immediate context is only used by mpv, which
// serializes it internally, and multithread protection is enabled below.
unsafe impl Send for VideoDevice {}
// SAFETY: see above.
unsafe impl Sync for VideoDevice {}

impl VideoDevice {
  /// Creates a device on adapter 0, the adapter GPUI picks, so shared handles open on both.
  pub fn new() -> windows::core::Result<Self> {
    // SAFETY: plain COM/DXGI calls with valid out-parameters.
    unsafe {
      let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
      let adapter = factory.EnumAdapters1(0)?;
      let mut device = None;
      let mut context = None;
      D3D11CreateDevice(
        &adapter,
        D3D_DRIVER_TYPE_UNKNOWN,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
        Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        Some(&mut context),
      )?;
      let device = device.ok_or_else(windows::core::Error::empty)?;
      let context = context.ok_or_else(windows::core::Error::empty)?;
      // mpv decodes (d3d11va) and renders on different threads with this device.
      if let Ok(mt) = device.cast::<ID3D11Multithread>() {
        let _ = mt.SetMultithreadProtected(true);
      }
      Ok(Self { device, context })
    }
  }

  /// Raw `ID3D11Device*` for `mpv_d3d11_init_params`.
  pub fn as_raw(&self) -> *mut c_void {
    self.device.as_raw()
  }

  fn texture_desc(width: u32, height: u32, shared: bool) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
      Width: width,
      Height: height,
      MipLevels: 1,
      ArraySize: 1,
      Format: DXGI_FORMAT_B8G8R8A8_UNORM,
      SampleDesc: DXGI_SAMPLE_DESC {
        Count: 1,
        Quality: 0,
      },
      Usage: D3D11_USAGE_DEFAULT,
      BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
      CPUAccessFlags: 0,
      MiscFlags: if shared {
        D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0 as u32
      } else {
        0
      },
    }
  }

  /// A private BGRA render target for mpv.
  pub fn create_target(&self, width: u32, height: u32) -> windows::core::Result<RenderTarget> {
    let desc = Self::texture_desc(width, height, false);
    // SAFETY: valid descriptor and out-parameter.
    unsafe {
      let mut texture = None;
      self
        .device
        .CreateTexture2D(&desc, None, Some(&mut texture))?;
      Ok(RenderTarget {
        texture: texture.ok_or_else(windows::core::Error::empty)?,
      })
    }
  }

  /// Copies `src` into `dst` on the GPU (same size and format).
  pub fn copy(&self, dst: &SharedTexture, src: &RenderTarget) {
    // SAFETY: both textures live on this device with identical descriptions; the immediate
    // context is multithread-protected.
    unsafe {
      self.context.CopyResource(&dst.texture, &src.texture);
      self.context.Flush();
    }
  }

  /// Creates a BGRA texture that another device can open through its shared handle.
  pub fn create_shared(&self, width: u32, height: u32) -> windows::core::Result<SharedTexture> {
    let desc = Self::texture_desc(width, height, true);
    // SAFETY: valid descriptor and out-parameter; the casts are COM QueryInterface calls.
    unsafe {
      let mut texture = None;
      self
        .device
        .CreateTexture2D(&desc, None, Some(&mut texture))?;
      let texture: ID3D11Texture2D = texture.ok_or_else(windows::core::Error::empty)?;
      let handle = texture.cast::<IDXGIResource>()?.GetSharedHandle()?;
      let mutex = texture.cast::<IDXGIKeyedMutex>()?;
      Ok(SharedTexture {
        texture,
        mutex,
        handle: handle.0 as usize as u64,
      })
    }
  }
}

/// A shared render target guarded by a keyed mutex (key 0 on both devices).
pub struct SharedTexture {
  texture: ID3D11Texture2D,
  mutex: IDXGIKeyedMutex,
  pub handle: u64,
}

// SAFETY: D3D11 resources are free-threaded; access to the contents is serialized by the keyed mutex.
unsafe impl Send for SharedTexture {}

impl SharedTexture {
  /// Acquires the keyed mutex. Returns false on timeout (WAIT_TIMEOUT is a success HRESULT,
  /// so the raw value is checked instead of `windows::core::Result`).
  pub fn acquire(&self, timeout_ms: u32) -> bool {
    // SAFETY: valid interface pointer, plain vtable call.
    let hr = unsafe {
      (Interface::vtable(&self.mutex).AcquireSync)(Interface::as_raw(&self.mutex), 0, timeout_ms)
    };
    hr.0 == 0
  }

  pub fn release(&self) {
    // SAFETY: only called after a successful `acquire`.
    let _ = unsafe { self.mutex.ReleaseSync(0) };
  }
}

/// A private render target (not shared): mpv renders here, then it is copied to a shared slot.
pub struct RenderTarget {
  texture: ID3D11Texture2D,
}

// SAFETY: D3D11 resources are free-threaded; only the render thread uses it.
unsafe impl Send for RenderTarget {}

impl RenderTarget {
  /// Raw `ID3D11Texture2D*` for `mpv_d3d11_fbo`.
  pub fn as_raw(&self) -> *mut c_void {
    self.texture.as_raw()
  }
}
