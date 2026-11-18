# Third-party notices

gevrek is published under the MIT licence (see `LICENSE`). It is built with
the work of many others, listed here with the licence each is used under.
Where a crate offers a choice of licences, gevrek uses it under the MIT
licence where that is offered, and otherwise under the Apache-2.0 licence.

## What gevrek stands on

| | What it is to gevrek | Licence |
|---|---|---|
| [Smithay](https://github.com/Smithay/smithay) | The Wayland compositor library under gevrek's Linux backend, and its client toolkit under the shell's surfaces | MIT |
| [egui and eframe](https://github.com/emilk/egui) | Everything gevrek draws: the studio, the panel, the launcher, the control centre, the lock screen | MIT OR Apache-2.0 |
| [wgpu](https://github.com/gfx-rs/wgpu) | The graphics device egui draws with | MIT OR Apache-2.0 |
| [windows-rs](https://github.com/microsoft/windows-rs) | The Windows API, for the Windows backend | MIT OR Apache-2.0 |
| [tonic](https://github.com/hyperium/tonic) and [prost](https://github.com/tokio-rs/prost) | The line between gevrek's programs | MIT, Apache-2.0 |
| [Phosphor Icons](https://phosphoricons.com), through [egui-phosphor](https://github.com/amPerl/egui-phosphor) | The icons of gevrek's programs | MIT |

## Fonts in the programs

gevrek's programs carry the fonts that egui carries
(`epaint_default_fonts`): **Ubuntu Light** (Ubuntu Font Licence 1.0),
**Hack** (MIT), **Noto Emoji** (SIL Open Font License 1.1) and
**emoji-icon-font** (MIT). Where the system has a font of its own for the
interface, gevrek reads that from the system and carries no copy of it.

## Its own marks

The gevrek logo, icons and boot pictures under `assets/` and `resources/`
are gevrek's own and are published with it under the MIT licence.

## Every crate

620 crates, as Cargo.lock has them. Not every one is in every program: some are
for one system only, some only build or test gevrek.

| Licence | Crates |
|---|---|
| MIT OR Apache-2.0 | 278 |
| MIT | 173 |
| Apache-2.0 OR MIT | 53 |
| Apache-2.0 | 23 |
| MIT/Apache-2.0 | 17 |
| Unlicense OR MIT | 11 |
| Zlib OR Apache-2.0 OR MIT | 10 |
| Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 9 |
| MIT OR Apache-2.0 OR Zlib | 6 |
| BSD-3-Clause | 4 |
| Zlib | 4 |
| Apache-2.0/MIT | 3 |
| BSL-1.0 | 2 |
| MIT OR Zlib OR Apache-2.0 | 2 |
| BSD-3-Clause OR Apache-2.0 | 2 |
| BSD-3-Clause OR MIT OR Apache-2.0 | 2 |
| MIT OR Apache-2.0 OR LGPL-2.1-or-later | 2 |
| Unlicense/MIT | 2 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 2 |
| BSD-2-Clause OR Apache-2.0 OR MIT | 2 |
| 0BSD OR MIT OR Apache-2.0 | 1 |
| BSD-2-Clause | 1 |
| Apache-2.0 OR MIT OR Unlicense | 1 |
| MIT / Apache-2.0 | 1 |
| Apache-2.0 AND MIT | 1 |
| (Apache-2.0 OR MIT) AND BSD-3-Clause | 1 |
| (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | 1 |
| Apache-2.0 / MIT | 1 |
| ISC | 1 |
| MIT AND BSD-3-Clause | 1 |
| MPL-2.0 | 1 |
| Apache-2.0 OR GPL-2.0-only | 1 |
| Apache-2.0 WITH LLVM-exception | 1 |

| Crate | Version | Licence | Source |
|---|---|---|---|
| ab_glyph | 0.2.32 | Apache-2.0 | <https://github.com/alexheretic/ab-glyph> |
| ab_glyph_rasterizer | 0.1.10 | Apache-2.0 | <https://github.com/alexheretic/ab-glyph> |
| accesskit | 0.24.1 | MIT OR Apache-2.0 | <https://github.com/AccessKit/accesskit> |
| accesskit | 0.25.2 | MIT OR Apache-2.0 | <https://github.com/AccessKit/accesskit> |
| accesskit_atspi_common | 0.21.1 | MIT OR Apache-2.0 | <https://github.com/AccessKit/accesskit> |
| accesskit_consumer | 0.39.2 | MIT OR Apache-2.0 | <https://github.com/AccessKit/accesskit> |
| accesskit_ios | 0.2.2 | MIT OR Apache-2.0 | <https://github.com/AccessKit/accesskit> |
| accesskit_macos | 0.27.2 | MIT OR Apache-2.0 | <https://github.com/AccessKit/accesskit> |
| accesskit_unix | 0.24.1 | MIT OR Apache-2.0 | <https://github.com/AccessKit/accesskit> |
| accesskit_windows | 0.35.2 | MIT OR Apache-2.0 | <https://github.com/AccessKit/accesskit> |
| accesskit_winit | 0.34.2 | Apache-2.0 | <https://github.com/AccessKit/accesskit> |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | <https://github.com/oyvindln/adler2> |
| ahash | 0.8.12 | MIT OR Apache-2.0 | <https://github.com/tkaitchuck/ahash> |
| aho-corasick | 1.1.5 | Unlicense OR MIT | <https://github.com/BurntSushi/aho-corasick> |
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 | <https://github.com/zakarumych/allocator-api2> |
| android-activity | 0.6.1 | MIT OR Apache-2.0 | <https://github.com/rust-mobile/android-activity> |
| android-properties | 0.2.2 | MIT | <https://github.com/miklelappo/android-properties> |
| android_system_properties | 0.1.6 | MIT OR Apache-2.0 | <https://github.com/nical/android_system_properties> |
| annotate-snippets | 0.11.5 | MIT OR Apache-2.0 | <https://github.com/rust-lang/annotate-snippets-rs> |
| anstream | 1.0.0 | MIT OR Apache-2.0 | <https://github.com/rust-cli/anstyle.git> |
| anstyle | 1.0.14 | MIT OR Apache-2.0 | <https://github.com/rust-cli/anstyle.git> |
| anstyle-parse | 1.0.0 | MIT OR Apache-2.0 | <https://github.com/rust-cli/anstyle.git> |
| anstyle-query | 1.1.5 | MIT OR Apache-2.0 | <https://github.com/rust-cli/anstyle.git> |
| anstyle-wincon | 3.0.11 | MIT OR Apache-2.0 | <https://github.com/rust-cli/anstyle.git> |
| anyhow | 1.0.104 | MIT OR Apache-2.0 | <https://github.com/dtolnay/anyhow> |
| appendlist | 1.4.0 | MIT | <https://github.com/danieldulaney/appendlist> |
| approx | 0.4.0 | Apache-2.0 | <https://github.com/brendanzab/approx> |
| arboard | 3.7.0 | MIT OR Apache-2.0 | <https://github.com/1Password/arboard> |
| arrayref | 0.3.9 | BSD-2-Clause | <https://github.com/droundy/arrayref> |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 | <https://github.com/bluss/arrayvec> |
| as-raw-xcb-connection | 1.0.1 | MIT OR Apache-2.0 | <https://github.com/psychon/as-raw-xcb-connection> |
| ash | 0.38.0+1.3.281 | MIT OR Apache-2.0 | <https://github.com/ash-rs/ash> |
| async-broadcast | 0.7.2 | MIT OR Apache-2.0 | <https://github.com/smol-rs/async-broadcast> |
| async-channel | 2.5.0 | Apache-2.0 OR MIT | <https://github.com/smol-rs/async-channel> |
| async-executor | 1.14.1 | Apache-2.0 OR MIT | <https://github.com/smol-rs/async-executor> |
| async-fs | 2.2.0 | Apache-2.0 OR MIT | <https://github.com/smol-rs/async-fs> |
| async-io | 2.6.0 | Apache-2.0 OR MIT | <https://github.com/smol-rs/async-io> |
| async-lock | 3.5.0 | Apache-2.0 OR MIT | <https://github.com/smol-rs/async-lock> |
| async-process | 2.5.0 | Apache-2.0 OR MIT | <https://github.com/smol-rs/async-process> |
| async-recursion | 1.2.0 | MIT OR Apache-2.0 | <https://github.com/dcchut/async-recursion> |
| async-signal | 0.2.14 | Apache-2.0 OR MIT | <https://github.com/smol-rs/async-signal> |
| async-stream | 0.3.6 | MIT | <https://github.com/tokio-rs/async-stream> |
| async-stream-impl | 0.3.6 | MIT | <https://github.com/tokio-rs/async-stream> |
| async-task | 4.7.1 | Apache-2.0 OR MIT | <https://github.com/smol-rs/async-task> |
| async-trait | 0.1.92 | MIT OR Apache-2.0 | <https://github.com/dtolnay/async-trait> |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT | <https://github.com/smol-rs/atomic-waker> |
| atomic_float | 1.1.0 | Apache-2.0 OR MIT OR Unlicense | <https://github.com/thomcc/atomic_float> |
| atspi | 0.29.0 | Apache-2.0 OR MIT | <https://github.com/odilia-app/atspi> |
| atspi-common | 0.13.0 | Apache-2.0 OR MIT | <https://github.com/odilia-app/atspi> |
| atspi-proxies | 0.13.0 | Apache-2.0 OR MIT | <https://github.com/odilia-app/atspi> |
| autocfg | 1.5.1 | Apache-2.0 OR MIT | <https://github.com/cuviper/autocfg> |
| axum | 0.7.9 | MIT | <https://github.com/tokio-rs/axum> |
| axum-core | 0.4.5 | MIT | <https://github.com/tokio-rs/axum> |
| base64 | 0.22.1 | MIT OR Apache-2.0 | <https://github.com/marshallpierce/rust-base64> |
| bindgen | 0.69.5 | BSD-3-Clause | <https://github.com/rust-lang/rust-bindgen> |
| bindgen | 0.72.1 | BSD-3-Clause | <https://github.com/rust-lang/rust-bindgen> |
| bit-set | 0.10.0 | Apache-2.0 OR MIT | <https://github.com/contain-rs/bit-set> |
| bit-vec | 0.9.1 | Apache-2.0 OR MIT | <https://github.com/contain-rs/bit-vec> |
| bitflags | 1.3.2 | MIT/Apache-2.0 | <https://github.com/bitflags/bitflags> |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | <https://github.com/bitflags/bitflags> |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 | <https://github.com/RustCrypto/utils> |
| block2 | 0.5.1 | MIT | <https://github.com/madsmtm/objc2> |
| block2 | 0.6.2 | MIT | <https://github.com/madsmtm/objc2> |
| blocking | 1.7.0 | Apache-2.0 OR MIT | <https://github.com/smol-rs/blocking> |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 | <https://github.com/fitzgen/bumpalo> |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT | <https://github.com/Lokathor/bytemuck> |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT | <https://github.com/Lokathor/bytemuck> |
| byteorder | 1.5.0 | Unlicense OR MIT | <https://github.com/BurntSushi/byteorder> |
| byteorder-lite | 0.1.0 | Unlicense OR MIT | <https://github.com/image-rs/byteorder-lite> |
| bytes | 1.12.1 | MIT | <https://github.com/tokio-rs/bytes> |
| calloop | 0.13.0 | MIT | <https://github.com/Smithay/calloop> |
| calloop | 0.14.5 | MIT | <https://github.com/Smithay/calloop> |
| calloop-wayland-source | 0.3.0 | MIT | <https://github.com/smithay/calloop-wayland-source> |
| cc | 1.11.1 | MIT OR Apache-2.0 | <https://github.com/rust-lang/cc-rs> |
| cexpr | 0.6.0 | Apache-2.0/MIT | <https://github.com/jethrogb/rust-cexpr> |
| cfg-expr | 0.20.11 | MIT OR Apache-2.0 | <https://github.com/EmbarkStudios/cfg-expr> |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | <https://github.com/rust-lang/cfg-if> |
| cfg_aliases | 0.2.2 | MIT | <https://github.com/katharostech/cfg_aliases> |
| cgl | 0.3.2 | MIT / Apache-2.0 | <https://github.com/servo/cgl-rs> |
| cgmath | 0.18.0 | Apache-2.0 | <https://github.com/rustgd/cgmath> |
| clang-sys | 1.9.1 | Apache-2.0 | <https://github.com/KyleMayes/clang-sys> |
| clap | 4.6.7 | MIT OR Apache-2.0 | <https://github.com/clap-rs/clap> |
| clap_builder | 4.6.7 | MIT OR Apache-2.0 | <https://github.com/clap-rs/clap> |
| clap_derive | 4.6.7 | MIT OR Apache-2.0 | <https://github.com/clap-rs/clap> |
| clap_lex | 1.1.1 | MIT OR Apache-2.0 | <https://github.com/clap-rs/clap> |
| clipboard-win | 5.4.3 | BSL-1.0 | <https://github.com/DoumanAsh/clipboard-win> |
| codespan-reporting | 0.13.1 | Apache-2.0 | <https://github.com/brendanzab/codespan> |
| color | 0.3.3 | Apache-2.0 OR MIT | <https://github.com/linebender/color> |
| color_quant | 1.1.0 | MIT | <https://github.com/image-rs/color_quant.git> |
| colorchoice | 1.0.5 | MIT OR Apache-2.0 | <https://github.com/rust-cli/anstyle.git> |
| combine | 4.6.9 | MIT | <https://github.com/Marwes/combine> |
| concurrent-queue | 2.5.0 | Apache-2.0 OR MIT | <https://github.com/smol-rs/concurrent-queue> |
| cookie-factory | 0.3.3 | MIT | <https://github.com/rust-bakery/cookie-factory> |
| core-foundation | 0.9.4 | MIT OR Apache-2.0 | <https://github.com/servo/core-foundation-rs> |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 | <https://github.com/servo/core-foundation-rs> |
| core-graphics | 0.23.2 | MIT OR Apache-2.0 | <https://github.com/servo/core-foundation-rs> |
| core-graphics-types | 0.1.3 | MIT OR Apache-2.0 | <https://github.com/servo/core-foundation-rs> |
| core_detect | 1.0.0 | MIT/Apache-2.0 | <https://github.com/thomcc/core_detect> |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 | <https://github.com/RustCrypto/utils> |
| crc32fast | 1.5.5 | MIT OR Apache-2.0 | <https://github.com/srijs/rust-crc32fast> |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 | <https://github.com/crossbeam-rs/crossbeam> |
| crunchy | 0.2.4 | MIT | <https://github.com/eira-fransham/crunchy> |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 | <https://github.com/RustCrypto/traits> |
| csscolorparser | 0.9.1 | MIT OR Apache-2.0 | <https://github.com/mazznoer/csscolorparser-rs> |
| ctrlc | 3.5.2 | MIT/Apache-2.0 | <https://github.com/Detegr/rust-ctrlc.git> |
| cursor-icon | 1.2.0 | MIT OR Apache-2.0 OR Zlib | <https://github.com/rust-windowing/cursor-icon> |
| data-url | 0.3.2 | MIT OR Apache-2.0 | <https://github.com/servo/rust-url> |
| digest | 0.10.7 | MIT OR Apache-2.0 | <https://github.com/RustCrypto/traits> |
| dirs | 5.0.1 | MIT OR Apache-2.0 | <https://github.com/soc/dirs-rs> |
| dirs-sys | 0.4.1 | MIT OR Apache-2.0 | <https://github.com/dirs-dev/dirs-sys-rs> |
| dispatch | 0.2.0 | MIT | <http://github.com/SSheldon/rust-dispatch> |
| dispatch2 | 0.3.1 | Zlib OR Apache-2.0 OR MIT | <https://github.com/madsmtm/objc2> |
| dlib | 0.5.3 | MIT | <https://github.com/elinorbgr/dlib> |
| document-features | 0.2.12 | MIT OR Apache-2.0 | <https://github.com/slint-ui/document-features> |
| downcast-rs | 1.2.1 | MIT/Apache-2.0 | <https://github.com/marcianx/downcast-rs> |
| dpi | 0.1.2 | Apache-2.0 AND MIT | <https://github.com/rust-windowing/winit> |
| drm | 0.14.1 | MIT | <https://github.com/Smithay/drm-rs> |
| drm-ffi | 0.9.1 | MIT | <https://github.com/Smithay/drm-rs> |
| drm-fourcc | 2.2.0 | MIT | <https://github.com/danielzfranklin/drm-fourcc-rs> |
| drm-sys | 0.8.1 | MIT | <https://github.com/Smithay/drm-rs> |
| dyn-clone | 1.0.20 | MIT OR Apache-2.0 | <https://github.com/dtolnay/dyn-clone> |
| ecolor | 0.36.2 | MIT OR Apache-2.0 | <https://github.com/emilk/egui> |
| ecolor | 0.37.0 | MIT OR Apache-2.0 | <https://github.com/emilk/egui> |
| eframe | 0.37.0 | MIT OR Apache-2.0 | <https://github.com/emilk/egui/tree/main/crates/eframe> |
| egui | 0.36.2 | MIT OR Apache-2.0 | <https://github.com/emilk/egui> |
| egui | 0.37.0 | MIT OR Apache-2.0 | <https://github.com/emilk/egui> |
| egui-phosphor | 0.14.0 | MIT OR Apache-2.0 | <https://github.com/amPerl/egui-phosphor> |
| egui-wgpu | 0.37.0 | MIT OR Apache-2.0 | <https://github.com/emilk/egui/tree/main/crates/egui-wgpu> |
| egui-winit | 0.37.0 | MIT OR Apache-2.0 | <https://github.com/emilk/egui/tree/main/crates/egui-winit> |
| egui_glow | 0.37.0 | MIT OR Apache-2.0 | <https://github.com/emilk/egui/tree/main/crates/egui_glow> |
| either | 1.19.0 | MIT OR Apache-2.0 | <https://github.com/rayon-rs/either> |
| emath | 0.36.2 | MIT OR Apache-2.0 | <https://github.com/emilk/egui/tree/main/crates/emath> |
| emath | 0.37.0 | MIT OR Apache-2.0 | <https://github.com/emilk/egui/tree/main/crates/emath> |
| embed-resource | 3.0.12 | MIT | <https://github.com/nabijaczleweli/rust-embed-resource> |
| encoding_rs | 0.8.44 | (Apache-2.0 OR MIT) AND BSD-3-Clause | <https://github.com/hsivonen/encoding_rs> |
| endi | 1.1.1 | MIT | <https://github.com/zeenix/endi> |
| enum-primitive-derive | 0.3.0 | MIT | <https://gitlab.com/cardoe/enum-primitive-derive.git> |
| enumflags2 | 0.7.12 | MIT OR Apache-2.0 | <https://github.com/meithecatte/enumflags2> |
| enumflags2_derive | 0.7.12 | MIT OR Apache-2.0 | <https://github.com/meithecatte/enumflags2> |
| env_filter | 2.0.0 | MIT OR Apache-2.0 | <https://github.com/rust-cli/env_logger> |
| env_logger | 0.11.11 | MIT OR Apache-2.0 | <https://github.com/rust-cli/env_logger> |
| epaint | 0.36.2 | MIT OR Apache-2.0 | <https://github.com/emilk/egui/tree/main/crates/epaint> |
| epaint | 0.37.0 | MIT OR Apache-2.0 | <https://github.com/emilk/egui/tree/main/crates/epaint> |
| epaint_default_fonts | 0.37.0 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | <https://github.com/emilk/egui/tree/main/crates/epaint_default_fonts> |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | <https://github.com/indexmap-rs/equivalent> |
| errno | 0.3.14 | MIT OR Apache-2.0 | <https://github.com/lambda-fairy/rust-errno> |
| error-code | 3.4.0 | BSL-1.0 | <https://github.com/DoumanAsh/error-code> |
| euclid | 0.22.14 | MIT OR Apache-2.0 | <https://github.com/servo/euclid> |
| event-listener | 5.4.2 | Apache-2.0 OR MIT | <https://github.com/smol-rs/event-listener> |
| event-listener-strategy | 0.5.4 | Apache-2.0 OR MIT | <https://github.com/smol-rs/event-listener-strategy> |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | <https://github.com/smol-rs/fastrand> |
| fax | 0.2.7 | MIT | <https://github.com/pdf-rs/fax> |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 | <https://github.com/image-rs/fdeflate> |
| fearless_simd | 0.4.1 | Apache-2.0 OR MIT | <https://github.com/linebender/fearless_simd> |
| find-msvc-tools | 0.1.15 | MIT OR Apache-2.0 | <https://github.com/rust-lang/cc-rs> |
| fixedbitset | 0.5.9 | MIT OR Apache-2.0 | <https://github.com/petgraph/fixedbitset> |
| flate2 | 1.1.11 | MIT OR Apache-2.0 | <https://github.com/rust-lang/flate2-rs> |
| float-cmp | 0.9.0 | MIT | <https://github.com/mikedilger/float-cmp> |
| fnv | 1.0.7 | Apache-2.0 / MIT | <https://github.com/servo/rust-fnv> |
| foldhash | 0.1.5 | Zlib | <https://github.com/orlp/foldhash> |
| foldhash | 0.2.0 | Zlib | <https://github.com/orlp/foldhash> |
| font-types | 0.12.7 | MIT OR Apache-2.0 | <https://github.com/googlefonts/fontations> |
| font8x8 | 0.3.1 | MIT | <https://gitlab.com/saibatizoku/font8x8-rs> |
| foreign-types | 0.5.0 | MIT/Apache-2.0 | <https://github.com/sfackler/foreign-types> |
| foreign-types-macros | 0.2.4 | MIT/Apache-2.0 | <https://github.com/sfackler/foreign-types> |
| foreign-types-shared | 0.3.1 | MIT/Apache-2.0 | <https://github.com/sfackler/foreign-types> |
| futures | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-executor | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-io | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-lite | 2.6.1 | Apache-2.0 OR MIT | <https://github.com/smol-rs/futures-lite> |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-task | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| gbm | 0.18.0 | MIT | <https://github.com/Smithay/gbm.rs> |
| gbm-sys | 0.4.0 | MIT | <https://github.com/Drakulix/gbm.rs/tree/master/gbm-sys> |
| generic-array | 0.14.7 | MIT | <https://github.com/fizyk20/generic-array.git> |
| gethostname | 1.1.0 | Apache-2.0 | <https://codeberg.org/swsnr/gethostname.rs.git> |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | <https://github.com/rust-random/getrandom> |
| getrandom | 0.3.4 | MIT OR Apache-2.0 | <https://github.com/rust-random/getrandom> |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | <https://github.com/rust-random/getrandom> |
| gif | 0.14.2 | MIT OR Apache-2.0 | <https://github.com/image-rs/image-gif> |
| gl_generator | 0.14.0 | Apache-2.0 | <https://github.com/brendanzab/gl-rs/> |
| glifo | 0.2.0 | Apache-2.0 OR MIT | <https://github.com/linebender/vello> |
| glifo | 0.3.0 | Apache-2.0 OR MIT | <https://github.com/linebender/vello> |
| glob | 0.3.4 | MIT OR Apache-2.0 | <https://github.com/rust-lang/glob> |
| glow | 0.17.0 | MIT OR Apache-2.0 OR Zlib | <https://github.com/grovesNL/glow> |
| glutin | 0.32.3 | Apache-2.0 | <https://github.com/rust-windowing/glutin> |
| glutin-winit | 0.5.0 | MIT | <https://github.com/rust-windowing/glutin> |
| glutin_egl_sys | 0.7.1 | Apache-2.0 | <https://github.com/rust-windowing/glutin> |
| glutin_glx_sys | 0.6.1 | Apache-2.0 | <https://github.com/rust-windowing/glutin> |
| glutin_wgl_sys | 0.6.1 | Apache-2.0 | <https://github.com/rust-windowing/glutin> |
| gpu-allocator | 0.28.0 | MIT OR Apache-2.0 | <https://github.com/Traverse-Research/gpu-allocator> |
| guillotiere | 0.7.0 | MIT/Apache-2.0 | <https://github.com/nical/guillotiere> |
| h2 | 0.4.20 | MIT | <https://github.com/hyperium/h2> |
| half | 2.7.1 | MIT OR Apache-2.0 | <https://github.com/VoidStarKat/half-rs> |
| harfrust | 0.12.0 | MIT | <https://github.com/harfbuzz/harfrust> |
| hashbrown | 0.12.3 | MIT OR Apache-2.0 | <https://github.com/rust-lang/hashbrown> |
| hashbrown | 0.15.5 | MIT OR Apache-2.0 | <https://github.com/rust-lang/hashbrown> |
| hashbrown | 0.16.1 | MIT OR Apache-2.0 | <https://github.com/rust-lang/hashbrown> |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | <https://github.com/rust-lang/hashbrown> |
| heck | 0.5.0 | MIT OR Apache-2.0 | <https://github.com/withoutboats/heck> |
| hermit-abi | 0.3.9 | MIT OR Apache-2.0 | <https://github.com/hermit-os/hermit-rs> |
| hermit-abi | 0.5.3 | MIT OR Apache-2.0 | <https://github.com/hermit-os/hermit-rs> |
| hex | 0.4.3 | MIT OR Apache-2.0 | <https://github.com/KokaKiwi/rust-hex> |
| home | 0.5.12 | MIT OR Apache-2.0 | <https://github.com/rust-lang/cargo> |
| http | 1.5.0 | MIT OR Apache-2.0 | <https://github.com/hyperium/http> |
| http-body | 1.1.0 | MIT | <https://github.com/hyperium/http-body> |
| http-body-util | 0.1.5 | MIT | <https://github.com/hyperium/http-body> |
| httparse | 1.10.1 | MIT OR Apache-2.0 | <https://github.com/seanmonstar/httparse> |
| httpdate | 1.0.3 | MIT OR Apache-2.0 | <https://github.com/pyfisch/httpdate> |
| hyper | 1.12.0 | MIT | <https://github.com/hyperium/hyper> |
| hyper-timeout | 0.5.2 | MIT OR Apache-2.0 | <https://github.com/hjr3/hyper-timeout> |
| hyper-util | 0.1.21 | MIT | <https://github.com/hyperium/hyper-util> |
| image | 0.25.10 | MIT OR Apache-2.0 | <https://github.com/image-rs/image> |
| image-webp | 0.2.4 | MIT OR Apache-2.0 | <https://github.com/image-rs/image-webp> |
| imagesize | 0.13.0 | MIT | <https://github.com/Roughsketch/imagesize> |
| indexmap | 1.9.3 | Apache-2.0 OR MIT | <https://github.com/bluss/indexmap> |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | <https://github.com/indexmap-rs/indexmap> |
| input | 0.9.1 | MIT | <https://github.com/Drakulix/input.rs> |
| input-sys | 1.19.0 | MIT | <https://github.com/Drakulix/input.rs/tree/master/input-sys> |
| io-lifetimes | 1.0.11 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | <https://github.com/sunfishcode/io-lifetimes> |
| is_terminal_polyfill | 1.70.2 | MIT OR Apache-2.0 | <https://github.com/polyfill-rs/is_terminal_polyfill> |
| itertools | 0.12.1 | MIT OR Apache-2.0 | <https://github.com/rust-itertools/itertools> |
| itertools | 0.14.0 | MIT OR Apache-2.0 | <https://github.com/rust-itertools/itertools> |
| itertools | 0.15.0 | MIT OR Apache-2.0 | <https://github.com/rust-itertools/itertools> |
| itoa | 1.0.19 | MIT OR Apache-2.0 | <https://github.com/dtolnay/itoa> |
| jiff | 0.2.43 | Unlicense OR MIT | <https://github.com/BurntSushi/jiff> |
| jiff-core | 0.1.5 | Unlicense OR MIT | <https://github.com/BurntSushi/jiff> |
| jiff-static | 0.2.43 | Unlicense OR MIT | <https://github.com/BurntSushi/jiff> |
| jiff-tzdb | 0.1.9 | Unlicense OR MIT | <https://github.com/BurntSushi/jiff> |
| jiff-tzdb-platform | 0.1.3 | Unlicense OR MIT | <https://github.com/BurntSushi/jiff> |
| jni | 0.22.4 | MIT OR Apache-2.0 | <https://github.com/jni-rs/jni-rs> |
| jni-macros | 0.22.4 | MIT OR Apache-2.0 | <https://github.com/jni-rs/jni-rs> |
| jni-sys | 0.3.1 | MIT OR Apache-2.0 | <https://github.com/jni-rs/jni-sys> |
| jni-sys | 0.4.1 | MIT OR Apache-2.0 | <https://github.com/jni-rs/jni-sys> |
| jni-sys-macros | 0.4.1 | MIT OR Apache-2.0 | <https://github.com/jni-rs/jni-sys> |
| jobserver | 0.1.35 | MIT OR Apache-2.0 | <https://github.com/rust-lang/jobserver-rs> |
| js-sys | 0.3.107 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys> |
| khronos-egl | 6.0.0 | MIT/Apache-2.0 | <https://github.com/timothee-haudebourg/khronos-egl> |
| khronos_api | 3.1.0 | Apache-2.0 | <https://github.com/brendanzab/gl-rs/> |
| kurbo | 0.11.3 | Apache-2.0 OR MIT | <https://github.com/linebender/kurbo> |
| kurbo | 0.13.1 | Apache-2.0 OR MIT | <https://github.com/linebender/kurbo> |
| lazy_static | 1.5.1 | MIT OR Apache-2.0 | <https://github.com/rust-lang-nursery/lazy-static.rs> |
| lazycell | 1.3.0 | MIT/Apache-2.0 | <https://github.com/indiv0/lazycell> |
| libc | 0.2.191 | MIT OR Apache-2.0 | <https://github.com/rust-lang/libc> |
| libloading | 0.8.9 | ISC | <https://github.com/nagisa/rust_libloading/> |
| libm | 0.2.16 | MIT | <https://github.com/rust-lang/compiler-builtins> |
| libredox | 0.1.25 | MIT | <https://gitlab.redox-os.org/redox-os/libredox.git> |
| libseat | 0.2.4 | MIT | <https://github.com/PolyMeilex/libseat-rs> |
| libseat-sys | 0.2.0 | MIT | <https://github.com/PolyMeilex/libseat-rs> |
| libspa | 0.10.1 | MIT | <https://gitlab.freedesktop.org/pipewire/pipewire-rs> |
| libspa-sys | 0.10.1 | MIT | <https://gitlab.freedesktop.org/pipewire/pipewire-rs> |
| libudev-sys | 0.1.4 | MIT | <https://github.com/dcuddeback/libudev-sys> |
| linebender_resource_handle | 0.1.1 | Apache-2.0 OR MIT | <https://github.com/linebender/raw_resource_handle> |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | <https://github.com/sunfishcode/linux-raw-sys> |
| linux-raw-sys | 0.4.15 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | <https://github.com/sunfishcode/linux-raw-sys> |
| linux-raw-sys | 0.9.4 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | <https://github.com/sunfishcode/linux-raw-sys> |
| litrs | 1.0.1 | MIT OR Apache-2.0 | <https://github.com/LukasKalbertodt/litrs> |
| lock_api | 0.4.14 | MIT OR Apache-2.0 | <https://github.com/Amanieu/parking_lot> |
| log | 0.4.35 | MIT OR Apache-2.0 | <https://github.com/rust-lang/log> |
| matchit | 0.7.3 | MIT AND BSD-3-Clause | <https://github.com/ibraheemdev/matchit> |
| memchr | 2.8.3 | Unlicense OR MIT | <https://github.com/BurntSushi/memchr> |
| memmap2 | 0.8.0 | MIT OR Apache-2.0 | <https://github.com/RazrFalcon/memmap2-rs> |
| memmap2 | 0.9.12 | MIT OR Apache-2.0 | <https://github.com/RazrFalcon/memmap2-rs> |
| memoffset | 0.9.1 | MIT | <https://github.com/Gilnaa/memoffset> |
| mime | 0.3.18 | MIT OR Apache-2.0 | <https://github.com/hyperium/mime> |
| minimal-lexical | 0.2.1 | MIT/Apache-2.0 | <https://github.com/Alexhuszagh/minimal-lexical> |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 | <https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide> |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 | <https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide> |
| mio | 1.2.4 | MIT | <https://github.com/tokio-rs/mio> |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 | <https://github.com/awxkee/moxcms.git> |
| multimap | 0.10.1 | MIT OR Apache-2.0 | <https://github.com/havarnov/multimap> |
| multiversion_no_op | 1.0.0 | Apache-2.0 OR MIT | <https://github.com/hsivonen/multiversion_no_op> |
| naga | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| naga-types | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| ndk | 0.9.0 | MIT OR Apache-2.0 | <https://github.com/rust-mobile/ndk> |
| ndk-context | 0.1.1 | MIT OR Apache-2.0 | <https://github.com/rust-windowing/android-ndk-rs> |
| ndk-sys | 0.6.0+11769913 | MIT OR Apache-2.0 | <https://github.com/rust-mobile/ndk> |
| nix | 0.29.0 | MIT | <https://github.com/nix-rust/nix> |
| nix | 0.31.3 | MIT | <https://github.com/nix-rust/nix> |
| nohash-hasher | 0.2.0 | Apache-2.0 OR MIT | <https://github.com/paritytech/nohash-hasher> |
| nom | 7.1.3 | MIT | <https://github.com/Geal/nom> |
| nom | 8.0.0 | MIT | <https://github.com/rust-bakery/nom> |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | <https://github.com/rust-num/num-traits> |
| num_enum | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 | <https://github.com/illicitonion/num_enum> |
| num_enum_derive | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 | <https://github.com/illicitonion/num_enum> |
| objc-sys | 0.3.5 | MIT | <https://github.com/madsmtm/objc2> |
| objc2 | 0.5.3 | MIT | <https://github.com/madsmtm/objc2> |
| objc2 | 0.6.5 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-app-kit | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-app-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT | <https://github.com/madsmtm/objc2> |
| objc2-cloud-kit | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-contacts | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-core-data | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-core-foundation | 0.3.2 | Zlib OR Apache-2.0 OR MIT | <https://github.com/madsmtm/objc2> |
| objc2-core-graphics | 0.3.2 | Zlib OR Apache-2.0 OR MIT | <https://github.com/madsmtm/objc2> |
| objc2-core-image | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-core-location | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-encode | 4.1.0 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-foundation | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-foundation | 0.3.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-io-surface | 0.3.2 | Zlib OR Apache-2.0 OR MIT | <https://github.com/madsmtm/objc2> |
| objc2-link-presentation | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-metal | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-metal | 0.3.2 | Zlib OR Apache-2.0 OR MIT | <https://github.com/madsmtm/objc2> |
| objc2-quartz-core | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-quartz-core | 0.3.2 | Zlib OR Apache-2.0 OR MIT | <https://github.com/madsmtm/objc2> |
| objc2-symbols | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-ui-kit | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-ui-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT | <https://github.com/madsmtm/objc2> |
| objc2-uniform-type-identifiers | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| objc2-user-notifications | 0.2.2 | MIT | <https://github.com/madsmtm/objc2> |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | <https://github.com/matklad/once_cell> |
| once_cell_polyfill | 1.70.2 | MIT OR Apache-2.0 | <https://github.com/polyfill-rs/once_cell_polyfill> |
| option-ext | 0.2.0 | MPL-2.0 | <https://github.com/soc/option-ext.git> |
| orbclient | 0.3.55 | MIT | <https://gitlab.redox-os.org/redox-os/orbclient> |
| ordered-float | 5.5.1 | MIT | <https://github.com/reem/rust-ordered-float> |
| ordered-stream | 0.2.0 | MIT OR Apache-2.0 | <https://github.com/danieldg/ordered-stream> |
| os_pipe | 1.2.3 | MIT | <https://github.com/oconnor663/os_pipe.rs> |
| owned_ttf_parser | 0.25.1 | Apache-2.0 | <https://github.com/alexheretic/owned-ttf-parser> |
| pam | 0.8.0 | MIT OR Apache-2.0 | <https://github.com/1wilkens/pam/> |
| pam-macros | 0.0.3 | MIT OR Apache-2.0 | <https://github.com/1wilkens/pam/> |
| pam-sys | 1.0.0-alpha5 | MIT OR Apache-2.0 | <https://github.com/1wilkens/pam-sys.git> |
| parking | 2.2.1 | Apache-2.0 OR MIT | <https://github.com/smol-rs/parking> |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 | <https://github.com/Amanieu/parking_lot> |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 | <https://github.com/Amanieu/parking_lot> |
| paste | 1.0.15 | MIT OR Apache-2.0 | <https://github.com/dtolnay/paste> |
| peniko | 0.6.1 | Apache-2.0 OR MIT | <https://github.com/linebender/peniko> |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | <https://github.com/servo/rust-url/> |
| petgraph | 0.7.1 | MIT OR Apache-2.0 | <https://github.com/petgraph/petgraph> |
| petgraph | 0.8.3 | MIT OR Apache-2.0 | <https://github.com/petgraph/petgraph> |
| phf | 0.14.0 | MIT | <https://github.com/rust-phf/rust-phf> |
| phf_generator | 0.14.0 | MIT | <https://github.com/rust-phf/rust-phf> |
| phf_macros | 0.14.0 | MIT | <https://github.com/rust-phf/rust-phf> |
| phf_shared | 0.14.0 | MIT | <https://github.com/rust-phf/rust-phf> |
| pico-args | 0.5.0 | MIT | <https://github.com/RazrFalcon/pico-args> |
| pin-project | 1.1.13 | Apache-2.0 OR MIT | <https://github.com/taiki-e/pin-project> |
| pin-project-internal | 1.1.13 | Apache-2.0 OR MIT | <https://github.com/taiki-e/pin-project> |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | <https://github.com/taiki-e/pin-project-lite> |
| piper | 0.2.5 | MIT OR Apache-2.0 | <https://github.com/smol-rs/piper> |
| pipewire | 0.10.1 | MIT | <https://gitlab.freedesktop.org/pipewire/pipewire-rs> |
| pipewire-sys | 0.10.1 | MIT | <https://gitlab.freedesktop.org/pipewire/pipewire-rs> |
| pixman | 0.2.1 | MIT | <https://github.com/cmeissl/pixman-rs> |
| pixman-sys | 0.1.0 | MIT | <https://github.com/cmeissl/pixman-rs> |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/pkg-config-rs> |
| plain | 0.2.3 | MIT/Apache-2.0 | <https://github.com/randomites/plain> |
| png | 0.17.16 | MIT OR Apache-2.0 | <https://github.com/image-rs/image-png> |
| png | 0.18.1 | MIT OR Apache-2.0 | <https://github.com/image-rs/image-png> |
| polling | 3.11.0 | Apache-2.0 OR MIT | <https://github.com/smol-rs/polling> |
| pollster | 1.0.1 | Apache-2.0/MIT | <https://github.com/zesterer/pollster> |
| polycool | 0.4.0 | MIT OR Apache-2.0 | <https://github.com/linebender/kurbo> |
| portable-atomic | 1.15.0 | Apache-2.0 OR MIT | <https://github.com/taiki-e/portable-atomic> |
| portable-atomic-util | 0.2.8 | Apache-2.0 OR MIT | <https://github.com/taiki-e/portable-atomic-util> |
| ppv-lite86 | 0.2.21 | MIT OR Apache-2.0 | <https://github.com/cryptocorrosion/cryptocorrosion> |
| presser | 0.3.1 | MIT OR Apache-2.0 | <https://github.com/EmbarkStudios/presser> |
| prettyplease | 0.2.37 | MIT OR Apache-2.0 | <https://github.com/dtolnay/prettyplease> |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 | <https://github.com/bkchr/proc-macro-crate> |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | <https://github.com/dtolnay/proc-macro2> |
| profiling | 1.0.18 | MIT OR Apache-2.0 | <https://github.com/aclysma/profiling> |
| profiling-procmacros | 1.0.18 | MIT OR Apache-2.0 | <https://github.com/aclysma/profiling> |
| prost | 0.13.5 | Apache-2.0 | <https://github.com/tokio-rs/prost> |
| prost-build | 0.13.5 | Apache-2.0 | <https://github.com/tokio-rs/prost> |
| prost-derive | 0.13.5 | Apache-2.0 | <https://github.com/tokio-rs/prost> |
| prost-types | 0.13.5 | Apache-2.0 | <https://github.com/tokio-rs/prost> |
| protoc-bin-vendored | 3.3.0 | MIT | <https://github.com/stepancheg/rust-protoc-bin-vendored/> |
| protoc-bin-vendored-linux-aarch_64 | 3.3.0 | MIT | <https://github.com/stepancheg/rust-protoc-bin-vendored/> |
| protoc-bin-vendored-linux-ppcle_64 | 3.3.0 | MIT | <https://github.com/stepancheg/rust-protoc-bin-vendored/> |
| protoc-bin-vendored-linux-s390_64 | 3.3.0 | MIT | <https://github.com/stepancheg/rust-protoc-bin-vendored/> |
| protoc-bin-vendored-linux-x86_32 | 3.3.0 | MIT | <https://github.com/stepancheg/rust-protoc-bin-vendored/> |
| protoc-bin-vendored-linux-x86_64 | 3.3.0 | MIT | <https://github.com/stepancheg/rust-protoc-bin-vendored/> |
| protoc-bin-vendored-macos-aarch_64 | 3.3.0 | MIT | <https://github.com/stepancheg/rust-protoc-bin-vendored/> |
| protoc-bin-vendored-macos-x86_64 | 3.3.0 | MIT | <https://github.com/stepancheg/rust-protoc-bin-vendored/> |
| protoc-bin-vendored-win32 | 3.3.0 | MIT | <https://github.com/stepancheg/rust-protoc-bin-vendored/> |
| pulseaudio | 0.3.2 | MIT | <https://github.com/colinmarc/pulseaudio-rs> |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 | <https://github.com/awxkee/pxfm> |
| quick-error | 2.0.1 | MIT/Apache-2.0 | <http://github.com/tailhook/quick-error> |
| quick-xml | 0.41.0 | MIT | <https://github.com/tafia/quick-xml> |
| quote | 1.0.47 | MIT OR Apache-2.0 | <https://github.com/dtolnay/quote> |
| r-efi | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | <https://github.com/r-efi/r-efi> |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | <https://github.com/r-efi/r-efi> |
| rand | 0.8.8 | MIT OR Apache-2.0 | <https://github.com/rust-random/rand> |
| rand | 0.9.5 | MIT OR Apache-2.0 | <https://github.com/rust-random/rand> |
| rand_chacha | 0.3.1 | MIT OR Apache-2.0 | <https://github.com/rust-random/rand> |
| rand_chacha | 0.9.0 | MIT OR Apache-2.0 | <https://github.com/rust-random/rand> |
| rand_core | 0.6.4 | MIT OR Apache-2.0 | <https://github.com/rust-random/rand> |
| rand_core | 0.9.5 | MIT OR Apache-2.0 | <https://github.com/rust-random/rand> |
| range-alloc | 0.1.5 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/range-alloc> |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib | <https://github.com/rust-windowing/raw-window-handle> |
| raw-window-metal | 1.1.0 | MIT OR Apache-2.0 | <https://github.com/rust-windowing/raw-window-metal> |
| read-fonts | 0.41.0 | MIT OR Apache-2.0 | <https://github.com/googlefonts/fontations> |
| redox_syscall | 0.4.1 | MIT | <https://gitlab.redox-os.org/redox-os/syscall> |
| redox_syscall | 0.5.18 | MIT | <https://gitlab.redox-os.org/redox-os/syscall> |
| redox_syscall | 0.9.5 | MIT | <https://gitlab.redox-os.org/redox-os/kernel> |
| redox_users | 0.4.6 | MIT | <https://gitlab.redox-os.org/redox-os/users> |
| ref-cast | 1.0.27 | MIT OR Apache-2.0 | <https://github.com/dtolnay/ref-cast> |
| ref-cast-impl | 1.0.27 | MIT OR Apache-2.0 | <https://github.com/dtolnay/ref-cast> |
| regex | 1.13.1 | MIT OR Apache-2.0 | <https://github.com/rust-lang/regex> |
| regex-automata | 0.4.18 | MIT OR Apache-2.0 | <https://github.com/rust-lang/regex> |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 | <https://github.com/rust-lang/regex> |
| renderdoc-sys | 1.1.0 | MIT OR Apache-2.0 | <https://github.com/ebkalderon/renderdoc-rs> |
| resvg | 0.45.1 | Apache-2.0 OR MIT | <https://github.com/linebender/resvg> |
| rgb | 0.8.53 | MIT | <https://github.com/kornelski/rust-rgb> |
| roxmltree | 0.20.0 | MIT OR Apache-2.0 | <https://github.com/RazrFalcon/roxmltree> |
| rustc-hash | 1.1.0 | Apache-2.0/MIT | <https://github.com/rust-lang-nursery/rustc-hash> |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT | <https://github.com/rust-lang/rustc-hash> |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 | <https://github.com/djc/rustc-version-rs> |
| rustix | 0.38.44 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | <https://github.com/bytecodealliance/rustix> |
| rustix | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | <https://github.com/bytecodealliance/rustix> |
| rustversion | 1.0.23 | MIT OR Apache-2.0 | <https://github.com/dtolnay/rustversion> |
| same-file | 1.0.6 | Unlicense/MIT | <https://github.com/BurntSushi/same-file> |
| schemars | 1.2.3 | MIT | <https://github.com/GREsau/schemars> |
| schemars_derive | 1.2.3 | MIT | <https://github.com/GREsau/schemars> |
| scoped-tls | 1.0.1 | MIT/Apache-2.0 | <https://github.com/alexcrichton/scoped-tls> |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 | <https://github.com/bluss/scopeguard> |
| self_cell | 1.3.0 | Apache-2.0 OR GPL-2.0-only | <https://github.com/Voultapher/self_cell> |
| semver | 1.0.28 | MIT OR Apache-2.0 | <https://github.com/dtolnay/semver> |
| send_wrapper | 0.6.0 | MIT/Apache-2.0 | <https://github.com/thk1/send_wrapper> |
| serde | 1.0.229 | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> |
| serde_derive_internals | 0.30.0 | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | <https://github.com/serde-rs/json> |
| serde_repr | 0.1.21 | MIT OR Apache-2.0 | <https://github.com/dtolnay/serde-repr> |
| serde_spanned | 1.1.3 | MIT OR Apache-2.0 | <https://github.com/toml-rs/toml> |
| sha1 | 0.10.7 | MIT OR Apache-2.0 | <https://github.com/RustCrypto/hashes> |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | <https://github.com/RustCrypto/hashes> |
| shlex | 1.3.0 | MIT OR Apache-2.0 | <https://github.com/comex/rust-shlex> |
| shlex | 2.0.1 | MIT OR Apache-2.0 | <https://github.com/comex/rust-shlex> |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 | <https://github.com/vorner/signal-hook> |
| simd-adler32 | 0.3.10 | MIT | <https://github.com/mcountryman/simd-adler32> |
| simd_cesu8 | 1.2.0 | Apache-2.0 OR MIT | <https://github.com/seancroach/simd_cesu8> |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 | <https://github.com/rusticstuff/simdutf8> |
| simplecss | 0.2.2 | Apache-2.0 OR MIT | <https://github.com/linebender/simplecss> |
| siphasher | 1.0.4 | MIT OR Apache-2.0 | <https://github.com/jedisct1/rust-siphash> |
| skrifa | 0.44.0 | MIT OR Apache-2.0 | <https://github.com/googlefonts/fontations> |
| slab | 0.4.12 | MIT | <https://github.com/tokio-rs/slab> |
| slotmap | 1.1.1 | Zlib | <https://github.com/orlp/slotmap> |
| smallvec | 1.16.5 | MIT OR Apache-2.0 | <https://github.com/servo/rust-smallvec> |
| smithay | 0.7.0 | MIT | <https://github.com/Smithay/smithay> |
| smithay-client-toolkit | 0.19.2 | MIT | <https://github.com/smithay/client-toolkit> |
| smol_str | 0.2.2 | MIT OR Apache-2.0 | <https://github.com/rust-analyzer/smol_str> |
| socket2 | 0.5.10 | MIT OR Apache-2.0 | <https://github.com/rust-lang/socket2> |
| socket2 | 0.6.5 | MIT OR Apache-2.0 | <https://github.com/rust-lang/socket2> |
| spirv | 0.4.0+sdk-1.4.341.0 | Apache-2.0 | <https://github.com/gfx-rs/rspirv> |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 | <https://github.com/nvzqz/static-assertions-rs> |
| strict-num | 0.1.1 | MIT | <https://github.com/RazrFalcon/strict-num> |
| strsim | 0.11.1 | MIT | <https://github.com/rapidfuzz/strsim-rs> |
| svgtypes | 0.15.3 | Apache-2.0 OR MIT | <https://github.com/linebender/svgtypes> |
| syn | 1.0.109 | MIT OR Apache-2.0 | <https://github.com/dtolnay/syn> |
| syn | 2.0.119 | MIT OR Apache-2.0 | <https://github.com/dtolnay/syn> |
| syn | 3.0.10 | MIT OR Apache-2.0 | <https://github.com/dtolnay/syn> |
| sync_wrapper | 1.0.2 | Apache-2.0 | <https://github.com/Actyx/sync_wrapper> |
| system-deps | 7.0.8 | MIT OR Apache-2.0 | <https://github.com/gdesmott/system-deps> |
| target-lexicon | 0.13.5 | Apache-2.0 WITH LLVM-exception | <https://github.com/bytecodealliance/target-lexicon> |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | <https://github.com/Stebalien/tempfile> |
| termcolor | 1.4.1 | Unlicense OR MIT | <https://github.com/BurntSushi/termcolor> |
| thiserror | 1.0.69 | MIT OR Apache-2.0 | <https://github.com/dtolnay/thiserror> |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | <https://github.com/dtolnay/thiserror> |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 | <https://github.com/dtolnay/thiserror> |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | <https://github.com/dtolnay/thiserror> |
| tiff | 0.11.3 | MIT | <https://github.com/image-rs/image-tiff> |
| tiny-skia | 0.11.4 | BSD-3-Clause | <https://github.com/RazrFalcon/tiny-skia> |
| tiny-skia-path | 0.11.4 | BSD-3-Clause | <https://github.com/RazrFalcon/tiny-skia/tree/master/path> |
| tokio | 1.54.1 | MIT | <https://github.com/tokio-rs/tokio> |
| tokio-macros | 2.7.2 | MIT | <https://github.com/tokio-rs/tokio> |
| tokio-stream | 0.1.19 | MIT | <https://github.com/tokio-rs/tokio> |
| tokio-util | 0.7.20 | MIT | <https://github.com/tokio-rs/tokio> |
| toml | 1.2.1+spec-1.1.0 | MIT OR Apache-2.0 | <https://github.com/toml-rs/toml> |
| toml_datetime | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 | <https://github.com/toml-rs/toml> |
| toml_edit | 0.25.19+spec-1.1.0 | MIT OR Apache-2.0 | <https://github.com/toml-rs/toml> |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 | <https://github.com/toml-rs/toml> |
| toml_writer | 1.2.0+spec-1.1.0 | MIT OR Apache-2.0 | <https://github.com/toml-rs/toml> |
| tonic | 0.12.3 | MIT | <https://github.com/hyperium/tonic> |
| tonic-build | 0.12.3 | MIT | <https://github.com/hyperium/tonic> |
| tower | 0.4.13 | MIT | <https://github.com/tower-rs/tower> |
| tower | 0.5.3 | MIT | <https://github.com/tower-rs/tower> |
| tower-layer | 0.3.3 | MIT | <https://github.com/tower-rs/tower> |
| tower-service | 0.3.3 | MIT | <https://github.com/tower-rs/tower> |
| tracing | 0.1.44 | MIT | <https://github.com/tokio-rs/tracing> |
| tracing-attributes | 0.1.31 | MIT | <https://github.com/tokio-rs/tracing> |
| tracing-core | 0.1.36 | MIT | <https://github.com/tokio-rs/tracing> |
| tree_magic_mini | 3.2.2 | MIT | <https://github.com/mbrubeck/tree_magic/> |
| try-lock | 0.2.5 | MIT | <https://github.com/seanmonstar/try-lock> |
| ttf-parser | 0.25.1 | MIT OR Apache-2.0 | <https://github.com/harfbuzz/ttf-parser> |
| type-map | 0.5.1 | MIT/Apache-2.0 | <https://github.com/kardeiz/type-map> |
| typenum | 1.20.1 | MIT OR Apache-2.0 | <https://github.com/paholg/typenum> |
| udev | 0.9.3 | MIT | <https://github.com/Smithay/udev-rs> |
| uds_windows | 1.2.1 | MIT | <https://github.com/haraldh/rust_uds_windows> |
| uncased | 0.9.10 | MIT OR Apache-2.0 | <https://github.com/SergioBenitez/uncased> |
| unicode-general-category | 1.1.0 | Apache-2.0 | <https://github.com/yeslogic/unicode-general-category> |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | <https://github.com/dtolnay/unicode-ident> |
| unicode-properties | 0.1.5 | (MIT OR Apache-2.0) AND Unicode-3.0 | <https://github.com/unicode-rs/unicode-properties> |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 | <https://github.com/unicode-rs/unicode-segmentation> |
| unicode-width | 0.2.3 | MIT OR Apache-2.0 | <https://github.com/unicode-rs/unicode-width> |
| users | 0.10.0 | MIT | <https://github.com/ogham/rust-users> |
| usvg | 0.45.1 | Apache-2.0 OR MIT | <https://github.com/linebender/resvg> |
| utf8parse | 0.2.2 | Apache-2.0 OR MIT | <https://github.com/alacritty/vte> |
| uuid | 1.29.2 | Apache-2.0 OR MIT | <https://github.com/uuid-rs/uuid> |
| vello_common | 0.1.0 | Apache-2.0 OR MIT | <https://github.com/linebender/vello> |
| vello_common | 0.2.0 | Apache-2.0 OR MIT | <https://github.com/linebender/vello> |
| vello_cpu | 0.1.0 | Apache-2.0 OR MIT | <https://github.com/linebender/vello> |
| vello_cpu | 0.2.0 | Apache-2.0 OR MIT | <https://github.com/linebender/vello> |
| version-compare | 0.2.1 | MIT | <https://gitlab.com/timvisee/version-compare> |
| version_check | 0.9.5 | MIT/Apache-2.0 | <https://github.com/SergioBenitez/version_check> |
| vswhom | 0.1.0 | MIT | <https://github.com/nabijaczleweli/vswhom.rs> |
| vswhom-sys | 0.1.3 | MIT | <https://github.com/nabijaczleweli/vswhom-sys.rs> |
| walkdir | 2.5.0 | Unlicense/MIT | <https://github.com/BurntSushi/walkdir> |
| want | 0.3.2 | MIT | <https://github.com/seanmonstar/want> |
| wasi | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | <https://github.com/bytecodealliance/wasi> |
| wasip2 | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | <https://github.com/bytecodealliance/wasi-rs> |
| wasm-bindgen | 0.2.130 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen> |
| wasm-bindgen-futures | 0.4.80 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures> |
| wasm-bindgen-macro | 0.2.130 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro> |
| wasm-bindgen-macro-support | 0.2.130 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/main/crates/macro-support> |
| wasm-bindgen-shared | 0.2.130 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared> |
| wayland-backend | 0.3.17 | MIT | <https://github.com/smithay/wayland-rs> |
| wayland-client | 0.31.15 | MIT | <https://github.com/smithay/wayland-rs> |
| wayland-csd-frame | 0.3.0 | MIT | <https://github.com/rust-windowing/wayland-csd-frame> |
| wayland-cursor | 0.31.14 | MIT | <https://github.com/smithay/wayland-rs> |
| wayland-egl | 0.32.11 | MIT | <https://github.com/smithay/wayland-rs> |
| wayland-protocols | 0.32.13 | MIT | <https://github.com/smithay/wayland-rs> |
| wayland-protocols-misc | 0.3.12 | MIT | <https://github.com/smithay/wayland-rs> |
| wayland-protocols-plasma | 0.3.12 | MIT | <https://github.com/smithay/wayland-rs> |
| wayland-protocols-wlr | 0.3.12 | MIT | <https://github.com/smithay/wayland-rs> |
| wayland-scanner | 0.31.11 | MIT | <https://github.com/smithay/wayland-rs> |
| wayland-server | 0.31.14 | MIT | <https://github.com/smithay/wayland-rs> |
| wayland-sys | 0.31.11 | MIT | <https://github.com/smithay/wayland-rs> |
| web-sys | 0.3.107 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys> |
| web-time | 1.1.0 | MIT OR Apache-2.0 | <https://github.com/daxpedda/web-time> |
| weezl | 0.1.12 | MIT OR Apache-2.0 | <https://github.com/image-rs/weezl> |
| wgpu | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-core | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-core-deps-apple | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-core-deps-emscripten | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-core-deps-wasm | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-core-deps-windows-linux-android | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-hal | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-naga-bridge | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-types | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| which | 4.4.2 | MIT | <https://github.com/harryfei/which-rs.git> |
| winapi-util | 0.1.11 | Unlicense OR MIT | <https://github.com/BurntSushi/winapi-util> |
| windows | 0.58.0 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows | 0.62.2 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-core | 0.58.0 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-core | 0.62.2 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-future | 0.3.2 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-implement | 0.58.0 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-interface | 0.58.0 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-result | 0.2.0 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-result | 0.4.1 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-strings | 0.1.0 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-sys | 0.48.0 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-targets | 0.48.5 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_aarch64_gnullvm | 0.48.5 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_aarch64_msvc | 0.48.5 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_i686_gnu | 0.48.5 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_i686_msvc | 0.48.5 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_x86_64_gnu | 0.48.5 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_x86_64_gnullvm | 0.48.5 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_x86_64_msvc | 0.48.5 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> |
| winit | 0.30.13 | Apache-2.0 | <https://github.com/rust-windowing/winit> |
| winnow | 1.0.5 | MIT | <https://github.com/winnow-rs/winnow> |
| winreg | 0.56.0 | MIT | <https://github.com/gentoo90/winreg-rs> |
| wit-bindgen | 0.57.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | <https://github.com/bytecodealliance/wit-bindgen> |
| wl-clipboard-rs | 0.9.5 | MIT OR Apache-2.0 | <https://github.com/YaLTeR/wl-clipboard-rs> |
| x11-dl | 2.21.0 | MIT | <https://github.com/AltF02/x11-rs.git> |
| x11rb | 0.13.2 | MIT OR Apache-2.0 | <https://github.com/psychon/x11rb> |
| x11rb-protocol | 0.13.2 | MIT OR Apache-2.0 | <https://github.com/psychon/x11rb> |
| xcursor | 0.3.11 | MIT | <https://github.com/esposm03/xcursor-rs> |
| xdg-home | 1.3.0 | MIT | <https://github.com/zeenix/xdg-home> |
| xkbcommon | 0.7.0 | MIT | <https://github.com/rust-x-bindings/xkbcommon-rs> |
| xkbcommon | 0.8.0 | MIT | <https://github.com/rust-x-bindings/xkbcommon-rs> |
| xkbcommon-dl | 0.4.2 | MIT | <https://github.com/rust-windowing/xkbcommon-dl> |
| xkeysym | 0.2.1 | MIT OR Apache-2.0 OR Zlib | <https://github.com/notgull/xkeysym> |
| xml-rs | 0.8.29 | MIT | <https://github.com/kornelski/xml-rs> |
| xmlwriter | 0.1.0 | MIT | <https://github.com/RazrFalcon/xmlwriter> |
| zbus | 4.4.0 | MIT | <https://github.com/dbus2/zbus/> |
| zbus | 5.19.0 | MIT | <https://github.com/z-galaxy/zbus/> |
| zbus-lockstep | 0.5.2 | MIT | <https://github.com/luukvanderduim/zbus-lockstep> |
| zbus-lockstep-macros | 0.5.2 | MIT | <https://github.com/luukvanderduim/zbus-lockstep> |
| zbus_macros | 4.4.0 | MIT | <https://github.com/dbus2/zbus/> |
| zbus_macros | 5.19.0 | MIT | <https://github.com/z-galaxy/zbus/> |
| zbus_names | 3.0.0 | MIT | <https://github.com/dbus2/zbus/> |
| zbus_names | 4.3.5 | MIT | <https://github.com/z-galaxy/zbus/> |
| zbus_xml | 5.2.1 | MIT | <https://github.com/z-galaxy/zbus/> |
| zcheapstr | 1.2.0 | MIT | <https://github.com/z-galaxy/zcheapstr/> |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | <https://github.com/google/zerocopy> |
| zerocopy-derive | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | <https://github.com/google/zerocopy> |
| zlib-rs | 0.6.8 | Zlib | <https://github.com/trifectatechfoundation/zlib-rs> |
| zmij | 1.0.23 | MIT | <https://github.com/dtolnay/zmij> |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib | <https://github.com/etemesi254/zune-image> |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib | <https://github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg> |
| zvariant | 4.2.0 | MIT | <https://github.com/dbus2/zbus/> |
| zvariant | 5.15.1 | MIT | <https://github.com/z-galaxy/zbus/> |
| zvariant_derive | 4.2.0 | MIT | <https://github.com/dbus2/zbus/> |
| zvariant_derive | 5.15.1 | MIT | <https://github.com/z-galaxy/zbus/> |
| zvariant_utils | 2.1.0 | MIT | <https://github.com/dbus2/zbus/> |
| zvariant_utils | 4.2.0 | MIT | <https://github.com/z-galaxy/zbus/> |
