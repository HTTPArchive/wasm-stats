/**
 * Copyright 2021 Google LLC
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::{io::Write, path::PathBuf};
use wasmbin::{
    builtins::Blob,
    sections::{ExportDesc, FuncBody, ImportDesc, Section},
    types::ValueType,
    visit::Visit,
};
use written_size::WrittenSize;

#[derive(Default, Debug, Serialize)]
struct ProposalStats {
    atomics: usize,
    ref_types: usize,
    simd: usize,
    tail_calls: usize,
    bulk: usize,
    multi_value: usize,
    non_trapping_conv: usize,
    sign_extend: usize,
    mutable_externals: usize,
    bigint_externals: usize,
}

#[derive(Serialize, Eq, PartialEq, Hash, Debug)]
enum Language {
    Rust,
    Emscripten,
    // a category for WebAssembly modules where there is some evidence that
    // it is Emscripten, but the methods used are not terribly reliable.
    LikelyEmscripten,
    AssemblyScript,
    Blazor,
    Unknown,
    Go,
}

impl Default for Language {
    fn default() -> Self {
        Language::Unknown
    }
}

#[derive(Default, Debug, Serialize)]
struct InstructionCategoryStats {
    load_store: usize,
    local_var: usize,
    global_var: usize,
    table: usize,
    memory: usize,
    control_flow: usize,
    direct_calls: usize,
    indirect_calls: usize,
    constants: usize,
    wait_notify: usize,
    other: usize,
}

#[derive(Default, Debug, Serialize)]
struct InstructionStats {
    total: usize,
    proposals: ProposalStats,
    categories: InstructionCategoryStats,
}

#[derive(Default, Debug, Serialize)]
struct SizeStats {
    code: usize,
    init: usize,
    externals: usize,
    types: usize,
    custom: usize,
    descriptors: usize,
    total: usize,
}

#[derive(Default, Debug, Serialize)]
struct ExternalStats {
    funcs: usize,
    memories: usize,
    globals: usize,
    tables: usize,
}

#[derive(Default, Debug, Serialize)]
struct Stats {
    funcs: usize,
    language: Language,
    instr: InstructionStats,
    size: SizeStats,
    imports: ExternalStats,
    exports: ExternalStats,
    custom_sections: Vec<String>,
    has_start: bool,
}

fn calc_size(wasm: &impl wasmbin::io::Encode) -> Result<usize> {
    let mut written_size = WrittenSize::new();
    wasm.encode(&mut written_size)?;
    Ok(written_size.size() as usize)
}

fn get_instruction_stats(funcs: &[Blob<FuncBody>]) -> Result<InstructionStats> {
    use wasmbin::instructions::{simd::SIMD, Instruction as I, Misc as M};

    let mut stats = InstructionStats::default();
    for func in funcs {
        let func = &func.try_contents()?.expr;
        stats.total += func.len();
        for i in func {
            match i {
                I::BlockStart(_)
                | I::LoopStart(_)
                | I::IfStart(_)
                | I::IfElse
                | I::End
                | I::Unreachable
                | I::Br(_)
                | I::BrIf(_)
                | I::BrTable { .. }
                | I::Return
                | I::Select
                | I::SelectWithTypes(_)
                | I::Nop
                | I::Drop => stats.categories.control_flow += 1,
                I::SIMD(i) => {
                    stats.proposals.simd += 1;
                    match i {
                        SIMD::V128Load(_)
                        | SIMD::V128Load8x8S(_)
                        | SIMD::V128Load8x8U(_)
                        | SIMD::V128Load16x4S(_)
                        | SIMD::V128Load16x4U(_)
                        | SIMD::V128Load32x2S(_)
                        | SIMD::V128Load32x2U(_)
                        | SIMD::V128Load8Splat(_)
                        | SIMD::V128Load16Splat(_)
                        | SIMD::V128Load32Splat(_)
                        | SIMD::V128Load64Splat(_)
                        | SIMD::V128Store(_)
                        | SIMD::V128Load8Lane(_, _)
                        | SIMD::V128Load16Lane(_, _)
                        | SIMD::V128Load32Lane(_, _)
                        | SIMD::V128Load64Lane(_, _)
                        | SIMD::V128Store8Lane(_, _)
                        | SIMD::V128Store16Lane(_, _)
                        | SIMD::V128Store32Lane(_, _)
                        | SIMD::V128Store64Lane(_, _) => stats.categories.load_store += 1,
                        SIMD::V128Const(_) => stats.categories.constants += 1,
                        _ => stats.categories.other += 1,
                    }
                }
                I::Atomic(i) => {
                    stats.proposals.atomics += 1;
                    match i {
                        wasmbin::instructions::Atomic::Wake(_)
                        | wasmbin::instructions::Atomic::I32Wait(_)
                        | wasmbin::instructions::Atomic::I64Wait(_) => {
                            stats.categories.wait_notify += 1;
                        }
                        wasmbin::instructions::Atomic::I32Load(_)
                        | wasmbin::instructions::Atomic::I64Load(_)
                        | wasmbin::instructions::Atomic::I32Load8U(_)
                        | wasmbin::instructions::Atomic::I32Load16U(_)
                        | wasmbin::instructions::Atomic::I64Load8U(_)
                        | wasmbin::instructions::Atomic::I64Load16U(_)
                        | wasmbin::instructions::Atomic::I64Load32U(_)
                        | wasmbin::instructions::Atomic::I32Store(_)
                        | wasmbin::instructions::Atomic::I64Store(_)
                        | wasmbin::instructions::Atomic::I32Store8(_)
                        | wasmbin::instructions::Atomic::I32Store16(_)
                        | wasmbin::instructions::Atomic::I64Store8(_)
                        | wasmbin::instructions::Atomic::I64Store16(_)
                        | wasmbin::instructions::Atomic::I64Store32(_) => {
                            stats.categories.load_store += 1;
                        }
                        _ => stats.categories.other += 1,
                    }
                }
                I::RefFunc(_) | I::RefIsNull | I::RefNull(_) => {
                    stats.proposals.ref_types += 1;
                    match i {
                        I::RefIsNull => stats.categories.other += 1,
                        _ => stats.categories.constants += 1,
                    }
                }
                I::Misc(i) => match i {
                    M::MemoryInit { .. }
                    | M::MemoryCopy { .. }
                    | M::MemoryFill(_)
                    | M::DataDrop(_) => {
                        stats.proposals.bulk += 1;
                        stats.categories.memory += 1;
                    }
                    M::TableInit { .. }
                    | M::TableCopy { .. }
                    | M::TableFill(_)
                    | M::ElemDrop(_) => {
                        stats.proposals.bulk += 1;
                        stats.categories.table += 1;
                    }
                    M::TableGrow(_) | M::TableSize(_) => {
                        stats.proposals.ref_types += 1;
                        stats.categories.table += 1;
                    }
                    M::I32TruncSatF32S
                    | M::I32TruncSatF32U
                    | M::I32TruncSatF64S
                    | M::I32TruncSatF64U
                    | M::I64TruncSatF32S
                    | M::I64TruncSatF32U
                    | M::I64TruncSatF64S
                    | M::I64TruncSatF64U => {
                        stats.proposals.non_trapping_conv += 1;
                        stats.categories.other += 1;
                    }
                },
                I::Call(_) => stats.categories.direct_calls += 1,
                I::CallIndirect(_) => stats.categories.indirect_calls += 1,
                I::ReturnCall(_) => {
                    stats.categories.control_flow += 1;
                    stats.categories.direct_calls += 1;
                    stats.proposals.tail_calls += 1;
                }
                I::ReturnCallIndirect(_) => {
                    stats.categories.control_flow += 1;
                    stats.categories.indirect_calls += 1;
                    stats.proposals.tail_calls += 1;
                }
                I::I32Const(_) | I::I64Const(_) | I::F32Const(_) | I::F64Const(_) => {
                    stats.categories.constants += 1
                }
                I::LocalGet(_) | I::LocalSet(_) | I::LocalTee(_) => {
                    stats.categories.local_var += 1;
                }
                I::GlobalGet(_) | I::GlobalSet(_) => {
                    stats.categories.global_var += 1;
                }
                I::TableGet(_) | I::TableSet(_) => {
                    stats.categories.table += 1;
                }
                I::I32Load(_)
                | I::I64Load(_)
                | I::F32Load(_)
                | I::F64Load(_)
                | I::I32Load8S(_)
                | I::I32Load8U(_)
                | I::I32Load16S(_)
                | I::I32Load16U(_)
                | I::I64Load8S(_)
                | I::I64Load8U(_)
                | I::I64Load16S(_)
                | I::I64Load16U(_)
                | I::I64Load32S(_)
                | I::I64Load32U(_)
                | I::I32Store(_)
                | I::I64Store(_)
                | I::F32Store(_)
                | I::F64Store(_)
                | I::I32Store8(_)
                | I::I32Store16(_)
                | I::I64Store8(_)
                | I::I64Store16(_)
                | I::I64Store32(_) => {
                    stats.categories.load_store += 1;
                }
                I::MemorySize(_) | I::MemoryGrow(_) => {
                    stats.categories.memory += 1;
                }
                I::I64ExtendI32U
                | I::I32Extend8S
                | I::I32Extend16S
                | I::I64Extend8S
                | I::I64Extend16S
                | I::I64Extend32S => {
                    stats.proposals.sign_extend += 1;
                    stats.categories.other += 1;
                }
                _ => {
                    stats.categories.other += 1;
                }
            }
        }
    }
    Ok(stats)
}

macro_rules! get_external_stats {
    ($section:expr, $ns:path) => {{
        use $ns::*;

        let mut stats = ExternalStats::default();

        for external in $section {
            match external.desc {
                Func(_) => stats.funcs += 1,
                Global(_) => stats.globals += 1,
                Mem(_) => stats.memories += 1,
                Table(_) => stats.tables += 1,
            }
        }

        stats
    }};
}

struct MaybeExternal<T> {
    pub value: T,
    pub is_external: bool,
}

impl<T> MaybeExternal<T> {
    fn external(self) -> Option<T> {
        if self.is_external {
            Some(self.value)
        } else {
            None
        }
    }
}

fn infer_language(module: &wasmbin::Module) -> Result<Language> {
    let mut imports = Vec::new();
    let mut exports = Vec::new();

    for section in &module.sections {
        match section {
            Section::Import(section) => {
                let section = section.try_contents()?;
                for import in section {
                    imports.push(&import.path);
                }
            }
            Section::Export(section) => {
                let section = section.try_contents()?;
                for export in section {
                    exports.push(export);
                }
            }
            _ => {}
        }
    }

    // NOTE: Need to check for Blazor ahead of Emscripten
    if imports.iter().any(|i| i.name.contains("blazor")) {
        return Ok(Language::Blazor);
    }

    if imports.iter().any(|i| i.name.contains("emscripten")) {
        return Ok(Language::Emscripten);
    }

    if imports.iter().any(|i| i.module == "go") {
        return Ok(Language::Go);
    }

    // these are all based on Rust using wasm-bindgen
    if imports.iter().any(|i| {
        i.name.contains("wbindgen")
            || i.name.contains("wbg")
            || i.module == "wbg"
            || i.module == "wbindgen"
    }) || exports.iter().any(|e| e.name.contains("wbindgen"))
    {
        return Ok(Language::Rust);
    }

    // Many of the wasm modules have been compressed with this very distinctive pattern. From looking at a number of wasm modules
    // and inspecting their contents, or the page that hosts them, it seems quite likely this is Emscripten. For example:
    //
    // https://tweet2doom.github.io/t2d-explorer.wasm
    //   => https://github.com/tweet2doom/tweet2doom.github.io - strong evidence of Emscripten
    //
    // https://graphonline.ru/script/Graphoffline.Emscripten.wasm - the clue is in the filename!
    //
    // https://wsr-starfinder.com/js/stellarium-web-engine.06229ae9.wasm
    //  => https://github.com/Stellarium/stellarium-web-engine - code makes reference to using Emscripten
    if (imports.iter().any(|i| i.module == "a" && i.name == "a")
        && imports.iter().any(|i| i.module == "a" && i.name == "b"))

    // another distinctive pattern, again, evidence suggests Emscripten
    // https://tx.me/
    // => https://github.com/Samsung/rlottie/blob/master/src/wasm/rlottiewasm.cpp - this is a cool project ;-)
    //
    // https://demo.harmonicvision.com - Emscripten mentioned in the page source
    //
    // https://webcamera.io - uses FFMpeg, which is an Emscripten project
    || (imports.iter().any(|i| i.module == "env" && i.name == "a")
        && imports.iter().any(|i| i.module == "env" && i.name == "b"))
    {
        return Ok(Language::LikelyEmscripten);
    }

    Ok(Language::Unknown)
}

fn get_stats(wasm: &[u8]) -> Result<Stats> {
    let m = wasmbin::Module::decode_from(wasm)?;
    let mut stats = Stats {
        size: SizeStats {
            total: wasm.len(),
            ..Default::default()
        },
        language: infer_language(&m)?,
        ..Default::default()
    };
    let mut global_types = Vec::new();
    let mut func_types = Vec::new();
    let mut types = &[] as &[_];
    for section in &m.sections {
        match section {
            Section::Custom(section) => {
                stats.size.custom += calc_size(section)?;
                stats
                    .custom_sections
                    .push(section.try_contents()?.name().to_owned());
            }
            Section::Type(section) => {
                stats.size.types += calc_size(section)?;
                types = section.try_contents()?;
                for ty in types {
                    if ty.results.len() > 1 {
                        stats.instr.proposals.multi_value += 1;
                    }
                }
            }
            Section::Import(section) => {
                stats.size.externals += calc_size(section)?;
                let section = section.try_contents()?;
                stats.imports = get_external_stats!(section, ImportDesc);
                for item in section {
                    match &item.desc {
                        ImportDesc::Global(ty) => {
                            global_types.push(MaybeExternal {
                                value: ty.clone(),
                                is_external: true,
                            });
                        }
                        ImportDesc::Func(type_id) => {
                            func_types.push(MaybeExternal {
                                value: *type_id,
                                is_external: true,
                            });
                        }
                        _ => {}
                    }
                }
            }
            Section::Function(section) => {
                stats.size.descriptors += calc_size(section)?;
                func_types.extend(section.try_contents()?.iter().map(|type_id| MaybeExternal {
                    value: *type_id,
                    is_external: false,
                }));
            }
            Section::Table(section) => {
                stats.size.descriptors += calc_size(section)?;
            }
            Section::Memory(section) => {
                stats.size.descriptors += calc_size(section)?;
                for ty in section.try_contents()? {
                    if ty.is_shared {
                        stats.instr.proposals.atomics += 1;
                    }
                }
            }
            Section::Global(section) => {
                stats.size.descriptors += calc_size(section)?;
                global_types.extend(section.try_contents()?.iter().map(|global| MaybeExternal {
                    value: global.ty.clone(),
                    is_external: false,
                }));
            }
            Section::Export(section) => {
                stats.size.externals += calc_size(section)?;
                let section = section.try_contents()?;
                stats.exports = get_external_stats!(section, ExportDesc);
                for item in section {
                    match item.desc {
                        ExportDesc::Global(global_id) => {
                            global_types[global_id.index as usize].is_external = true;
                        }
                        ExportDesc::Func(func_id) => {
                            func_types[func_id.index as usize].is_external = true;
                        }
                        _ => {}
                    }
                }
            }
            Section::Start(_) => {
                stats.has_start = true;
            }
            Section::Element(section) => {
                stats.size.init += calc_size(section)?;
            }
            Section::DataCount(_) => {
                stats.instr.proposals.bulk += 1;
            }
            Section::Code(section) => {
                stats.size.code = calc_size(section)?;
                let funcs = section.try_contents()?;
                stats.funcs = funcs.len();
                stats.instr = get_instruction_stats(funcs)?;
            }
            Section::Data(section) => {
                stats.size.init += calc_size(section)?;
            }
        }
    }
    global_types
        .into_iter()
        .filter_map(MaybeExternal::external)
        .for_each(|ty| {
            if ty.mutable {
                stats.instr.proposals.mutable_externals += 1;
            }
            if let ValueType::I64 = ty.value_type {
                stats.instr.proposals.bigint_externals += 1;
            }
        });
    func_types
        .into_iter()
        .filter_map(MaybeExternal::external)
        .try_for_each(|type_id| {
            types[type_id.index as usize].visit(|ty: &ValueType| {
                if let ValueType::I64 = ty {
                    stats.instr.proposals.bigint_externals += 1;
                }
            })
        })?;
    Ok(stats)
}

fn main() -> Result<()> {
    let path_str = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow!("Please provide wasm file path"))?;
    let path = PathBuf::from(&path_str);
    let abs_path = std::fs::canonicalize(&path)?;
    let wasm = std::fs::read(&abs_path)?;
    let stats = get_stats(&wasm)?;
    let serialized = serde_json::to_string(&stats)? + "\n";
    std::io::stdout().write_all(serialized.as_bytes())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats_from_wat(wat: &str) -> Result<Stats> {
        let binary = wat::parse_str(wat)?;
        get_stats(&binary[..])
    }

    #[test]
    fn get_stats_funcs() -> Result<()> {
        let stats = stats_from_wat(
            r#"
        (module
            (func $foo)
            (func (export "bar")
                call $foo
            )
        )
        "#,
        )?;
        // TODO: test more of the stats
        assert_eq!(stats.funcs, 2);
        Ok(())
    }

    #[test]
    fn infer_language_unknown() -> Result<()> {
        let stats = stats_from_wat("(module)")?;
        assert_eq!(stats.language, Language::Unknown);
        Ok(())
    }

    #[test]
    fn infer_language_rust() -> Result<()> {
        // b63e9f90187a9f5cec9a7a9cfc15e68e9330979ac39a29258282c020780cd6ec.wasm
        //
        // exports mention 'wbindgen'
        let stats = stats_from_wat(
            r#"
        (module
            (type $t8 (func (param i32)))
            (func $__wbindgen_malloc (type $t8) (param $p0 i32))
            (export "__wbindgen_malloc" (func $__wbindgen_malloc))
        )
        "#,
        )?;
        assert_eq!(stats.language, Language::Rust);

        // 82f052ee941598c3f70b9adfdebcb8fda239e5095e48d3e4a2edcc208b0c769c.wasm
        //
        // import references a 'wbg' module
        let stats = stats_from_wat(
            r#"
        (module
            (type $t2 (func (param i32)))
            (import "wbg" "__wbindgen_object_drop_ref" (func $wasm_bindgen::__wbindgen_object_drop_ref::hc5b72d1598c36103 (type $t2)))
        )
        "#,
        )?;
        assert_eq!(stats.language, Language::Rust);

        // d792c9bfa765ab3e849bb2f266e1d2b19e555fc4a59c51d22a47fa73b27180b8.wasm
        //
        // import references a function containing 'wbg'
        let stats = stats_from_wat(
            r#"
        (module
            (type $t11 (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
            (import "./source_compiler_bg.js" "__wbg_sourcerorLogCallback_9555c6dd7a1fa2a1" (func $./source_compiler_bg.js.__wbg_sourcerorLogCallback_9555c6dd7a1fa2a1 (type $t11)))
        )
        "#,
        )?;
        assert_eq!(stats.language, Language::Rust);
        Ok(())
    }

    #[test]
    fn infer_language_blazor() -> Result<()> {
        // 9bd69204e55c94eb68b385ed4f79dffc752dc8fbccd526fd5c61d13a5df5d5de.wasm
        let stats = stats_from_wat(
            r#"
        (module
            (type $t4 (func (param i32 i32 i32) (result i32)))
            (type $t8 (func (param i32 i32 i32 i32 i32) (result i32)))
            (import "env" "mono_wasm_invoke_js_blazor" (func $env.mono_wasm_invoke_js_blazor (type $t8)))
            (import "env" "emscripten_asm_const_int" (func $env.emscripten_asm_const_int (type $t4)))
        )
        "#,
        )?;
        assert_eq!(stats.language, Language::Blazor);
        Ok(())
    }

    #[test]
    fn infer_language_emscripten() -> Result<()> {
        // 70c2f8e0269dd409da3153196ee3e4258f196d313ea271b1516c7fc241c52adb.wasm
        let stats = stats_from_wat(
            r#"
        (module
            (type $t3 (func (param i32) (result i32)))
            (import "env" "_emscripten_asm_const_i" (func $env._emscripten_asm_const_i (type $t3)))
        )
        "#,
        )?;
        assert_eq!(stats.language, Language::Emscripten);
        Ok(())
    }

    #[test]
    fn infer_language_go() -> Result<()> {
        // 1b98798659012dc524343d1a44da2488fb09436fd6ca587c804ad272367d294d.wasm
        let stats = stats_from_wat(
            r#"
        (module
            (type $t1 (func (param i32)))
            (import "go" "runtime.resetMemoryDataView" (func $go.runtime.resetMemoryDataView (type $t1)))
        )
        "#,
        )?;
        assert_eq!(stats.language, Language::Go);
        Ok(())
    }

    #[test]
    fn infer_language_likely_emscripten() -> Result<()> {
        // 38049c6cc89d4c6ac8c2635fc0af29901109d68247ba7e57d2bff551216a322e.wasm
        let stats = stats_from_wat(
            r#"
        (module
            (type $t4 (func (param i32 i32 i32) (result i32)))
            (import "a" "a" (func $a.a (type $t4)))
            (import "a" "b" (func $a.b (type $t4)))
        )
        "#,
        )?;
        assert_eq!(stats.language, Language::LikelyEmscripten);

        // f50ed354fd14cce39533af5fc58c0e4387a326748114c57a2ce3c98611da673b.wasm
        let stats = stats_from_wat(
            r#"
        (module
            (type $t6 (func (param i32 i32 i32 i32)))
            (import "env" "b" (func $env.b (type $t6)))
            (import "env" "a" (global $env.a i32))
        )
        "#,
        )?;
        assert_eq!(stats.language, Language::LikelyEmscripten);

        Ok(())
    }
}
