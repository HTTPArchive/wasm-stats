# wasm-stats

A command line tool used for [WebAssembly analysis in Web Almanac](https://almanac.httparchive.org/en/2021/webassembly).

Run the command line tool, providing a single WebAssembly module as an agument. The results are returned in JSON format. Here's an example:

```bash
$ cargo run --release -- module.wasm
{"funcs":44687,"instr":{"total":6359312,"proposals":{"atomics":0,"ref_types":0,"simd":0,"tail_calls":0,"bulk":0,"multi_value":0,"non_trapping_conv":0,"sign_extend":1372,"mutable_externals":0,"bigint_externals":0},"categories":{"load_store":996805,"local_var":2332199,"global_var":117428,"table":0,"memory":1,"control_flow":669774,"direct_calls":233176,"indirect_calls":20700,"constants":1019207,"wait_notify":0,"other":970022}},"size":{"code":14056337,"init":1676227,"externals":25838,"types":6434,"custom":0,"descriptors":46242,"total":15811094},"imports":{"funcs":408,"memories":1,"globals":6,"tables":1},"exports":{"funcs":500,"memories":0,"globals":0,"tables":0},"custom_sections":[],"has_start":false}
```

## language inference

wasm-stats profiles the wasm modules in an attempt to determine the original source language. This is not an exact science! Some are easy to spot, e.g. mention of specific technologies in imports / exports, whereas others are harder to determine.

The methods used in this tool have been tested on a recent crawl (with ~1,000 modules), and the inference techniques developed manually / iteratively. They have been developed within the following project: https://github.com/ColinEberhardt/wasm-lang-inference
