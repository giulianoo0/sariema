// SPDX-License-Identifier: Apache-2.0

//! WebGL2 runs shaders as GLSL ES 3.00, which lacks some of what WGSL and the
//! native backends allow, and binds uniforms in whole 16-byte rows. Every
//! shader the downlevel path makes is translated here as wgpu translates it
//! on WebGL2, so a shader that only a browser would refuse fails natively.

use naga::back::glsl;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use sariema_format::actor::demo::demo_arm;
use sariema_format::demo::demo_room;

use crate::{Headless, Quality, RenderPath};

fn webgl2_errors(label: &str, source: &str) -> Vec<String> {
    let module = match naga::front::wgsl::parse_str(source) {
        Ok(m) => m,
        Err(e) => return vec![format!("{label}: {}", e.emit_to_string(source))],
    };
    let info = match Validator::new(ValidationFlags::all(), Capabilities::empty()).validate(&module)
    {
        Ok(i) => i,
        Err(e) => return vec![format!("{label}: {}", e.emit_to_string(source))],
    };
    let mut errors = Vec::new();
    for (_, global) in module.global_variables.iter() {
        if global.space == naga::AddressSpace::Uniform {
            let size = module.types[global.ty].inner.size(module.to_ctx());
            if size % 16 != 0 {
                errors.push(format!(
                    "{label}: uniform {:?} is {size} bytes",
                    global.name
                ));
            }
        }
    }
    let options = glsl::Options {
        version: glsl::Version::Embedded {
            version: 300,
            is_webgl: true,
        },
        ..Default::default()
    };
    for entry in &module.entry_points {
        let stage = Some((entry.stage, entry.name.as_str()));
        let (module, info) = naga::back::pipeline_constants::process_overrides(
            &module,
            &info,
            stage,
            &Default::default(),
        )
        .unwrap();
        let pipeline = glsl::PipelineOptions {
            shader_stage: entry.stage,
            entry_point: entry.name.clone(),
            multiview: None,
        };
        let mut out = String::new();
        let written = glsl::Writer::new(
            &mut out,
            &module,
            &info,
            &options,
            &pipeline,
            Default::default(),
        )
        .and_then(|mut w| w.write().map(drop));
        if let Err(e) = written {
            errors.push(format!("{label} {}: {e}", entry.name));
        }
    }
    errors
}

fn downlevel_shaders(msaa: u32) -> Vec<(String, String)> {
    let quality = Quality {
        path: RenderPath::Downlevel,
        msaa,
        ..Quality::high()
    };
    let mut headless = Headless::new(64, 64, quality).unwrap();
    let renderer = headless.renderer();
    let mut room = demo_room();
    room.look.ao_strength = 1.0;
    renderer.load_room(&room).unwrap();
    renderer.load_actor(&demo_arm()).unwrap();
    renderer.gpu().shaders.borrow().clone()
}

#[test]
fn every_downlevel_shader_translates_to_webgl2() {
    let mut shaders = downlevel_shaders(4);
    shaders.extend(downlevel_shaders(1));
    shaders.push(("ui".into(), crate::ui::SHADER.into()));
    for label in [
        "scene",
        "actor",
        "actor shadow",
        "shadow",
        "resolve depth",
        "composite",
    ] {
        assert!(shaders.iter().any(|(l, _)| l == label), "no {label} shader");
    }
    let errors: Vec<String> = (shaders.iter())
        .flat_map(|(label, source)| webgl2_errors(label, source))
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn a_query_glsl_es_lacks_is_caught() {
    let source = "@group(0) @binding(0) var t: texture_2d<f32>;
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4(f32(textureNumLevels(t))); }";
    let errors = webgl2_errors("levels", source);
    assert!(
        errors.iter().any(|e| e.contains("TEXTURE_LEVELS")),
        "{errors:?}"
    );
    let source = "struct P { a: f32, b: f32 }
        @group(0) @binding(0) var<uniform> p: P;
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4(p.a + p.b); }";
    let errors = webgl2_errors("uniform", source);
    assert!(errors.iter().any(|e| e.contains("8 bytes")), "{errors:?}");
}
