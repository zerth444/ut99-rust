use std::collections::HashMap;

use windows::Win32::Foundation::{DXGI_STATUS_OCCLUDED, E_FAIL, HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob, ID3DInclude,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{Error, Interface, PCSTR, Result, s};

use crate::render::commands::{DrawList, Image, LIGHTMAPS, Layer, Material, Mode, Vertex};

const CHAIN_FLAGS: DXGI_SWAP_CHAIN_FLAG = DXGI_SWAP_CHAIN_FLAG(0);

pub struct Presenter {
    chain: IDXGISwapChain1,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    pipeline: Pipeline,
    targets: Option<Targets>,
    textures: HashMap<usize, Option<ID3D11ShaderResourceView>>,
    vertices: Option<ID3D11Buffer>,
    capacity: usize,
    batches: Vec<(usize, Material, u32, u32)>,
    size: (u32, u32),
    pub software: bool,
}

struct Targets {
    color: [Option<ID3D11RenderTargetView>; 1],
    depth: ID3D11DepthStencilView,
}

impl Presenter {
    pub fn new(window: windows_sys::Win32::Foundation::HWND, size: (u32, u32)) -> Result<Self> {
        let (device, software) = match create_device(D3D_DRIVER_TYPE_HARDWARE) {
            Ok(device) => (device, false),
            Err(_) => (create_device(D3D_DRIVER_TYPE_WARP)?, true),
        };
        let dxgi = device.cast::<IDXGIDevice1>()?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: size.0,
            Height: size.1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: CHAIN_FLAGS.0 as u32,
            ..Default::default()
        };
        let window = HWND(window);
        // The immediate context, HWND and DXGI presentation stay on the message-pump thread.
        let (chain, context) = unsafe {
            dxgi.SetMaximumFrameLatency(1)?;
            let factory = dxgi.GetAdapter()?.GetParent::<IDXGIFactory2>()?;
            let chain = factory.CreateSwapChainForHwnd(
                &device,
                window,
                &desc,
                None,
                None::<&IDXGIOutput>,
            )?;
            factory.MakeWindowAssociation(window, DXGI_MWA_NO_ALT_ENTER)?;
            (chain, device.GetImmediateContext()?)
        };
        let pipeline = Pipeline::new(&device)?;
        let targets = Some(Targets::new(&device, &chain, size)?);
        Ok(Self {
            chain,
            device,
            context,
            pipeline,
            targets,
            textures: HashMap::new(),
            vertices: None,
            capacity: 0,
            batches: Vec::new(),
            size,
            software,
        })
    }

    pub fn draw(&mut self, size: (u32, u32), commands: &mut DrawList) -> Result<bool> {
        self.resize(size)?;
        for (id, image, mips) in commands.uploads.drain(..) {
            let texture = self.upload(&image, mips)?;
            self.textures.insert(id, Some(texture));
        }
        self.stream_vertices(commands)?;
        let targets = self.targets.as_ref().ok_or_else(Error::empty)?;
        let viewport = D3D11_VIEWPORT {
            Width: size.0 as f32,
            Height: size.1 as f32,
            MaxDepth: 1.0,
            ..Default::default()
        };
        let context = &self.context;
        let pipeline = &self.pipeline;
        unsafe {
            context.OMSetRenderTargets(Some(&targets.color), &targets.depth);
            context.ClearRenderTargetView(
                targets.color[0].as_ref().unwrap(),
                &[13.0 / 255.0, 21.0 / 255.0, 32.0 / 255.0, 1.0],
            );
            context.ClearDepthStencilView(&targets.depth, D3D11_CLEAR_DEPTH.0 as u32, 0.0, 0);
            context.RSSetViewports(Some(&[viewport]));
            context.RSSetState(&pipeline.raster);
            context.IASetInputLayout(&pipeline.layout);
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.IASetVertexBuffers(
                0,
                1,
                Some(&self.vertices),
                Some(&(size_of::<Vertex>() as u32)),
                Some(&0),
            );
            context.VSSetShader(&pipeline.vertex, None);
            context.PSSetShader(&pipeline.pixel, None);
            context
                .PSSetShaderResources(1, self.textures.get(&LIGHTMAPS).map(std::slice::from_ref));
            let mut layer = Layer::Sky as usize;
            let mut mode = None;
            let mut texture = None;
            for &(pass, material, first, count) in &self.batches {
                if pass != layer
                    && (pass == Layer::World as usize || pass == Layer::Weapon as usize)
                {
                    context.ClearDepthStencilView(
                        &targets.depth,
                        D3D11_CLEAR_DEPTH.0 as u32,
                        0.0,
                        0,
                    );
                }
                layer = pass;
                if mode != Some(material.mode) {
                    pipeline.bind(context, material.mode);
                    mode = Some(material.mode);
                }
                if texture != Some(material.texture) {
                    context.PSSetShaderResources(
                        0,
                        self.textures
                            .get(&material.texture)
                            .map(std::slice::from_ref),
                    );
                    texture = Some(material.texture);
                }
                context.Draw(count, first);
            }
            let status = self.chain.Present(1, DXGI_PRESENT(0));
            status.ok()?;
            Ok(status != DXGI_STATUS_OCCLUDED)
        }
    }

    fn resize(&mut self, size: (u32, u32)) -> Result<()> {
        if self.size != size {
            unsafe {
                self.context
                    .OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
                self.targets = None;
                self.chain
                    .ResizeBuffers(0, size.0, size.1, DXGI_FORMAT_UNKNOWN, CHAIN_FLAGS)?;
            }
            self.targets = Some(Targets::new(&self.device, &self.chain, size)?);
            self.size = size;
        }
        Ok(())
    }

    fn stream_vertices(&mut self, commands: &DrawList) -> Result<()> {
        let count: usize = commands
            .passes
            .iter()
            .flat_map(|pass| pass.batches())
            .map(|(_, v)| v.len())
            .sum();
        self.batches.clear();
        if count == 0 {
            return Ok(());
        }
        if count > self.capacity {
            let capacity = count.next_power_of_two();
            let bytes =
                u32::try_from(capacity * size_of::<Vertex>()).map_err(|_| Error::empty())?;
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: bytes,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut buffer = None;
            unsafe {
                self.device.CreateBuffer(&desc, None, Some(&mut buffer))?;
            }
            self.vertices = buffer;
            self.capacity = capacity;
        }
        let buffer = self.vertices.as_ref().ok_or_else(Error::empty)?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            let target = mapped.pData.cast::<Vertex>();
            let mut offset = 0;
            for (layer, pass) in commands.passes.iter().enumerate() {
                for (material, vertices) in pass.batches() {
                    target
                        .add(offset)
                        .copy_from_nonoverlapping(vertices.as_ptr(), vertices.len());
                    self.batches
                        .push((layer, material, offset as u32, vertices.len() as u32));
                    offset += vertices.len();
                }
            }
            self.context.Unmap(buffer, 0);
        }
        Ok(())
    }

    fn upload(&self, image: &Image, mips: bool) -> Result<ID3D11ShaderResourceView> {
        let mut texture = None;
        let mut view = None;
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: image.pixels.as_ptr().cast(),
            SysMemPitch: image.width * 4,
            SysMemSlicePitch: 0,
        };
        let desc = D3D11_TEXTURE2D_DESC {
            Width: image.width,
            Height: image.height,
            MipLevels: u32::from(!mips),
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0
                | if mips { D3D11_BIND_RENDER_TARGET.0 } else { 0 }) as u32,
            MiscFlags: if mips {
                D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32
            } else {
                0
            },
            ..Default::default()
        };
        unsafe {
            self.device
                .CreateTexture2D(&desc, (!mips).then_some(&data), Some(&mut texture))?;
            let texture = texture.ok_or_else(Error::empty)?;
            self.device
                .CreateShaderResourceView(&texture, None, Some(&mut view))?;
            let view = view.ok_or_else(Error::empty)?;
            if mips {
                self.context.UpdateSubresource(
                    &texture,
                    0,
                    None,
                    data.pSysMem,
                    data.SysMemPitch,
                    0,
                );
                self.context.GenerateMips(&view);
            }
            Ok(view)
        }
    }
}

impl Targets {
    fn new(device: &ID3D11Device, chain: &IDXGISwapChain1, (w, h): (u32, u32)) -> Result<Self> {
        let mut color = None;
        let mut depth = None;
        let mut depth_view = None;
        unsafe {
            let back = chain.GetBuffer::<ID3D11Texture2D>(0)?;
            device.CreateRenderTargetView(&back, None, Some(&mut color))?;
            device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: w,
                    Height: h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_D32_FLOAT,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    BindFlags: D3D11_BIND_DEPTH_STENCIL.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut depth),
            )?;
            device.CreateDepthStencilView(
                &depth.ok_or_else(Error::empty)?,
                None,
                Some(&mut depth_view),
            )?;
        }
        Ok(Self {
            color: [color],
            depth: depth_view.ok_or_else(Error::empty)?,
        })
    }
}

struct Pipeline {
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    raster: ID3D11RasterizerState,
    world: [Option<ID3D11SamplerState>; 2],
    ui: [Option<ID3D11SamplerState>; 2],
    depth_write: ID3D11DepthStencilState,
    depth_read: ID3D11DepthStencilState,
    depth_off: ID3D11DepthStencilState,
    screen: ID3D11BlendState,
    no_color: ID3D11BlendState,
}

impl Pipeline {
    fn new(device: &ID3D11Device) -> Result<Self> {
        let vertex_code = shader(s!("vertex_main"), s!("vs_4_0"))?;
        let pixel_code = shader(s!("pixel_main"), s!("ps_4_0"))?;
        let attr = |semantic, index, format, offset| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: semantic,
            SemanticIndex: index,
            Format: format,
            AlignedByteOffset: offset,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            ..Default::default()
        };
        let attrs = [
            attr(s!("POSITION"), 0, DXGI_FORMAT_R32G32B32_FLOAT, 0),
            attr(s!("TEXCOORD"), 0, DXGI_FORMAT_R32G32_FLOAT, 12),
            attr(s!("TEXCOORD"), 1, DXGI_FORMAT_R32G32_FLOAT, 20),
            attr(s!("TEXCOORD"), 2, DXGI_FORMAT_R32G32B32A32_FLOAT, 28),
            attr(s!("COLOR"), 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 44),
        ];
        let mut vertex = None;
        let mut pixel = None;
        let mut layout = None;
        let mut raster = None;
        let mut screen = None;
        let mut no_color = None;
        unsafe {
            device.CreateVertexShader(
                &vertex_code,
                None::<&ID3D11ClassLinkage>,
                Some(&mut vertex),
            )?;
            device.CreatePixelShader(&pixel_code, None::<&ID3D11ClassLinkage>, Some(&mut pixel))?;
            device.CreateInputLayout(&attrs, &vertex_code, Some(&mut layout))?;
            device.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    DepthClipEnable: true.into(),
                    ..Default::default()
                },
                Some(&mut raster),
            )?;
            let mut blend = D3D11_BLEND_DESC::default();
            blend.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_ONE,
                DestBlend: D3D11_BLEND_INV_SRC_COLOR,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_ONE,
                DestBlendAlpha: D3D11_BLEND_ZERO,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            device.CreateBlendState(&blend, Some(&mut screen))?;
            blend.RenderTarget[0].BlendEnable = false.into();
            blend.RenderTarget[0].RenderTargetWriteMask = 0;
            device.CreateBlendState(&blend, Some(&mut no_color))?;
        }
        let light = sampler(
            device,
            D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT,
            D3D11_TEXTURE_ADDRESS_CLAMP,
        )?;
        Ok(Self {
            vertex: vertex.ok_or_else(Error::empty)?,
            pixel: pixel.ok_or_else(Error::empty)?,
            layout: layout.ok_or_else(Error::empty)?,
            raster: raster.ok_or_else(Error::empty)?,
            world: [
                Some(sampler(
                    device,
                    D3D11_FILTER_ANISOTROPIC,
                    D3D11_TEXTURE_ADDRESS_WRAP,
                )?),
                Some(light.clone()),
            ],
            ui: [
                Some(sampler(
                    device,
                    D3D11_FILTER_MIN_MAG_MIP_POINT,
                    D3D11_TEXTURE_ADDRESS_CLAMP,
                )?),
                Some(light),
            ],
            depth_write: depth_state(device, true, true)?,
            depth_read: depth_state(device, true, false)?,
            depth_off: depth_state(device, false, false)?,
            screen: screen.ok_or_else(Error::empty)?,
            no_color: no_color.ok_or_else(Error::empty)?,
        })
    }

    fn bind(&self, context: &ID3D11DeviceContext, mode: Mode) {
        let (depth, blend, samplers) = match mode {
            Mode::World => (&self.depth_write, None, &self.world),
            Mode::SkyMask => (&self.depth_write, Some(&self.no_color), &self.world),
            Mode::ScreenDepth => (&self.depth_read, Some(&self.screen), &self.world),
            Mode::Screen => (&self.depth_off, Some(&self.screen), &self.ui),
            Mode::Ui => (&self.depth_off, None, &self.ui),
        };
        unsafe {
            context.OMSetDepthStencilState(depth, 0);
            context.OMSetBlendState(blend, None, u32::MAX);
            context.PSSetSamplers(0, Some(samplers));
        }
    }
}

fn shader(entry: PCSTR, target: PCSTR) -> Result<Vec<u8>> {
    let source = include_bytes!("render/scene.hlsl");
    let mut output: Option<ID3DBlob> = None;
    let mut diagnostics: Option<ID3DBlob> = None;
    unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            s!("scene.hlsl"),
            None,
            None::<&ID3DInclude>,
            entry,
            target,
            0,
            0,
            &mut output,
            Some(&mut diagnostics),
        )
        .map_err(|err| {
            diagnostics.map_or(err, |blob| {
                let bytes = std::slice::from_raw_parts(
                    blob.GetBufferPointer().cast(),
                    blob.GetBufferSize(),
                );
                Error::new(
                    E_FAIL,
                    String::from_utf8_lossy(bytes).trim_end_matches('\0'),
                )
            })
        })?;
        let blob = output.ok_or_else(Error::empty)?;
        Ok(
            std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize())
                .to_vec(),
        )
    }
}

fn sampler(
    device: &ID3D11Device,
    filter: D3D11_FILTER,
    address: D3D11_TEXTURE_ADDRESS_MODE,
) -> Result<ID3D11SamplerState> {
    let mut state = None;
    unsafe {
        device.CreateSamplerState(
            &D3D11_SAMPLER_DESC {
                Filter: filter,
                AddressU: address,
                AddressV: address,
                AddressW: address,
                MaxAnisotropy: 8,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                MaxLOD: f32::MAX,
                ..Default::default()
            },
            Some(&mut state),
        )?;
    }
    state.ok_or_else(Error::empty)
}

fn depth_state(
    device: &ID3D11Device,
    enabled: bool,
    write: bool,
) -> Result<ID3D11DepthStencilState> {
    let mut state = None;
    unsafe {
        device.CreateDepthStencilState(
            &D3D11_DEPTH_STENCIL_DESC {
                DepthEnable: enabled.into(),
                DepthFunc: D3D11_COMPARISON_GREATER_EQUAL,
                DepthWriteMask: if write {
                    D3D11_DEPTH_WRITE_MASK_ALL
                } else {
                    D3D11_DEPTH_WRITE_MASK_ZERO
                },
                ..Default::default()
            },
            Some(&mut state),
        )?;
    }
    state.ok_or_else(Error::empty)
}

fn create_device(driver: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    device.ok_or_else(Error::empty)
}
