// SPDX-License-Identifier: Apache-2.0

//! The canvas's GPU: WebGPU where the browser has it and the page allows,
//! else WebGL2, chosen once when the engine is created.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};
use web_sys::HtmlCanvasElement;

use crate::settings::{Backend, WebSettings};
use crate::stage::StageError;

pub struct Gpu {
    pub surface: wgpu::Surface<'static>,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub config: wgpu::SurfaceConfiguration,
    pub info: wgpu::AdapterInfo,
    /// Set when the device is lost; the page then makes a new engine.
    pub lost: Arc<AtomicBool>,
}

impl Gpu {
    pub async fn new(canvas: HtmlCanvasElement, settings: &WebSettings) -> Result<Gpu, StageError> {
        let backend = settings.backend;
        let gpu_error = |why: String| StageError::new("E_GPU", why);
        let backends = match backend {
            Backend::Auto => wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
            Backend::WebGpu => wgpu::Backends::BROWSER_WEBGPU,
            Backend::WebGl2 => wgpu::Backends::GL,
        };
        let descriptor = wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        };
        let instance = wgpu::util::new_instance_with_webgpu_detection(descriptor).await;
        let (width, height) = (canvas.width().max(1), canvas.height().max(1));
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|e| gpu_error(format!("the canvas gave no surface: {e}")))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| gpu_error(format!("no {backend:?} adapter: {e}")))?;
        let compressed = wgpu::Features::TEXTURE_COMPRESSION_ASTC
            | wgpu::Features::TEXTURE_COMPRESSION_BC
            | wgpu::Features::TEXTURE_COMPRESSION_ETC2;
        let mut wanted = wgpu::Features::TIMESTAMP_QUERY;
        if settings.compressed_textures {
            wanted |= compressed;
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("sariema web"),
                required_features: adapter.features() & wanted,
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .map_err(|e| gpu_error(format!("the adapter gave no device: {e}")))?;
        let lost = Arc::new(AtomicBool::new(false));
        let flag = lost.clone();
        device.set_device_lost_callback(move |reason, why| {
            web_sys::console::warn_1(&format!("sariema: device lost ({reason:?}): {why}").into());
            flag.store(true, Ordering::Relaxed);
        });
        device.on_uncaptured_error(Arc::new(move |e| {
            web_sys::console::error_1(&format!("sariema: {e}").into());
        }));
        let caps = surface.get_capabilities(&adapter);
        // The composite encodes sRGB itself into a linear target.
        let format = (caps.formats.iter())
            .copied()
            .find(|f| !f.is_srgb())
            .or(caps.formats.first().copied())
            .ok_or_else(|| gpu_error("the canvas offers no format".into()))?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: Vec::new(),
            color_space: Default::default(),
        };
        surface.configure(&device, &config);
        Ok(Gpu {
            surface,
            device: Arc::new(device),
            queue: Arc::new(queue),
            config,
            info: adapter.get_info(),
            lost,
        })
    }

    pub fn backend(&self) -> &'static str {
        match self.info.backend {
            wgpu::Backend::BrowserWebGpu => "webgpu",
            wgpu::Backend::Gl => "webgl2",
            _ => "other",
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }

    /// What the page's probe and stats report about the device.
    pub fn describe(&self) -> Value {
        let l = self.device.limits();
        let f = self.device.features();
        let has = |feature| f.contains(feature);
        json!({
            "backend": self.backend(),
            "adapter": self.info.name,
            "driver": self.info.driver,
            "format": format!("{:?}", self.config.format),
            "astc": has(wgpu::Features::TEXTURE_COMPRESSION_ASTC),
            "bc": has(wgpu::Features::TEXTURE_COMPRESSION_BC),
            "etc2": has(wgpu::Features::TEXTURE_COMPRESSION_ETC2),
            "timestamps": has(wgpu::Features::TIMESTAMP_QUERY),
            "limits": {
                "max_texture_dimension_2d": l.max_texture_dimension_2d,
                "max_storage_buffers_per_shader_stage": l.max_storage_buffers_per_shader_stage,
                "max_uniform_buffer_binding_size": l.max_uniform_buffer_binding_size,
                "max_vertex_attributes": l.max_vertex_attributes,
                "max_bind_groups": l.max_bind_groups,
                "max_color_attachments": l.max_color_attachments,
            },
        })
    }
}
