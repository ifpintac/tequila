//! Every WGSL module compiles, for Vulkan and for Metal, on any machine
//! (`docs/07` R2.1): naga validates it and writes it out as SPIR-V and as
//! Metal's shading language, so a shader that does not compile fails CI on
//! a machine with no graphics card. The translations are left in the
//! system's temporary folder, `tequila-wgsl`, to read.

use tequila_render_gpu::shader;

#[test]
fn every_module_compiles_for_vulkan_and_metal() {
    let out = std::env::temp_dir().join("tequila-wgsl");
    std::fs::create_dir_all(&out).expect("a folder for the translations");
    for (name, _) in shader::MODULES {
        let source = shader::source(name);
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{name}.wgsl does not parse:\n{}", e.emit_to_string(&source)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}.wgsl does not validate:\n{}", e.emit_to_string(&source)));
        let spirv = naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), None)
            .unwrap_or_else(|e| panic!("{name}.wgsl has no SPIR-V: {e}"));
        std::fs::write(out.join(format!("{name}.spv")), bytemuck::cast_slice::<u32, u8>(&spirv)).expect("writes");
        let options = naga::back::msl::Options { lang_version: (2, 4), ..Default::default() };
        let (metal, _) = naga::back::msl::write_string(&module, &info, &options, &naga::back::msl::PipelineOptions::default())
            .unwrap_or_else(|e| panic!("{name}.wgsl has no Metal: {e}"));
        std::fs::write(out.join(format!("{name}.metal")), metal).expect("writes");
    }
}
