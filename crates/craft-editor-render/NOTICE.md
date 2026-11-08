# Native renderer third-party material

CraftEditor code is MIT. Its dependency and embedded-font licenses remain independently applicable. This notice records source provenance; it does not relicense upstream code or fonts. Distributors must retain the complete feature-aware Cargo notices and the applicable upstream license texts.

## Merman and its ELK implementation

Merman 0.9.3 is MIT OR Apache-2.0. Source release: [`3b915f69c02d55b873d252cf6c8a6048e7f3427d`](https://github.com/Latias94/merman/tree/3b915f69c02d55b873d252cf6c8a6048e7f3427d).

The selected complete feature closure includes `merman-elk-layered`, whose translated Eclipse ELK source is EPL-2.0. Its [source and implementation boundary](https://github.com/Latias94/merman/tree/3b915f69c02d55b873d252cf6c8a6048e7f3427d/crates/merman-elk-layered) are separate from Merman's MIT/Apache code. The unmodified governing text is retained at [third_party/EPL-2.0.txt](third_party/EPL-2.0.txt).

Upstream records these derivation baselines:

- [Eclipse ELK `62d5909f96fad541bc101ad52dabaece6b7eab7e`](https://github.com/eclipse-elk/elk/tree/62d5909f96fad541bc101ad52dabaece6b7eab7e).
- [elkjs `a8304cf79fde75bc2ab1a89d28320f53f8637436`](https://github.com/kieler/elkjs/tree/a8304cf79fde75bc2ab1a89d28320f53f8637436).
- [Historical Mermaid ELK adapter `41646dfd43ac83f001b03c70605feb036afae46d`](https://github.com/mermaid-js/mermaid/blob/41646dfd43ac83f001b03c70605feb036afae46d/packages/mermaid-layout-elk/src/render.ts).

These are translation provenance points, not a claim that CraftEditor has independently qualified every Mermaid family or configuration.

## RaTeX and KaTeX fonts

RaTeX 0.1.15 is MIT; source release [`5f7dc4883aba8eeb30fe34f8a104923f43f734bf`](https://github.com/erweixin/RaTeX/tree/5f7dc4883aba8eeb30fe34f8a104923f43f734bf). The selected `embed-fonts` feature embeds KaTeX font data distributed by `ratex-katex-fonts`, which remains under SIL Open Font License 1.1. Unmodified upstream material is retained at [third_party/KaTeX-OFL.txt](third_party/KaTeX-OFL.txt) and [third_party/KaTeX-FONT_NOTICE.txt](third_party/KaTeX-FONT_NOTICE.txt).

## WordCraft text fonts

The native SVG database uses `wordcraft_fonts::bundled()` at source commit `da914ce674f18ebc4e08cca425cc8f8acca3372f`. It embeds Source Sans 3, Source Serif 4, Inter and JetBrains Mono font faces under their original OFL licenses. No font files are copied into this repository. The source licenses are retained by WordCraft under [`assets/fonts`](https://github.com/storytold/wordcraft/tree/da914ce674f18ebc4e08cca425cc8f8acca3372f/assets/fonts); consuming application packages must include these notices alongside their runtime assets.

resvg/usvg 0.47.0 and image 0.25.10 are MIT OR Apache-2.0 and remain listed in Cargo's resolved dependency inventory.
