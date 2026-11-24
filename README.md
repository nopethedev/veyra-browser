# Veyra

An independent desktop web browser written in Rust. Veyra ships its **own** networking,
HTML parser, CSS engine, layout engine, rasterizer and JavaScript interpreter — no
Chromium, Gecko, WebKit, Servo, Electron, Tauri, CEF, V8, SpiderMonkey or QuickJS
anywhere in the dependency tree.

The whole engine is about 28,000 lines of Rust across ten crates plus a CLI/window shell.

```
┌──────────── veyra-shell (minifb window, tabs, chrome, shortcuts)
│  ┌───────── veyra-engine (page lifecycle, DOM bindings, events, fetch/cookies)
│  │  ┌────── veyra-js (own lexer → parser → interpreter → builtins)
│  │  ├────── veyra-html → veyra-dom (tokenizer, tree construction, selectors)
│  │  ├────── veyra-css (parser, cascade, computed style, media queries)
│  │  ├────── veyra-layout (block/inline/flex/grid, fonts, text shaping)
│  │  └────── veyra-paint (software RGBA rasterizer, gradients, images, shadows)
│  └───────── veyra-net (HTTP/1.1 over rustls, cookies, cache, redirects, gzip)
└──────────── veyra-common (colors, geometry, URLs, JSON)
```

## Current status

| Area | Status |
| --- | --- |
| Linux build + tests | **Working** — `cargo test --workspace` green, release binary 9.4 MB |
| Windows build | **Cross-compiled** — `dist/veyra-windows-x86_64.exe` (PE32+, 8.5 MB). Not executed on Windows in this environment. |
| Real HTTPS pages | **Working** — rustls + webpki roots, redirects, gzip/deflate, chunked encoding, cookies, disk cache |
| Modern CSS (compiled) | **Mostly working** — flexbox, grid, gradients, custom properties, `calc()`, shadows, transforms, `background-clip: text`, media queries |
| Tailwind (compiled stylesheet / CDN CSS) | **Working** — verified with Tailwind v2 CDN and Tailwind v4 precompiled output |
| Tailwind Play CDN (runtime JIT compiler) | **Working** — the unpatched CDN compiles its stylesheet inside Veyra and the page renders styled, with no script errors |
| JavaScript | **Substantial subset** — ES5.1 plus much of ES2015–ES2022 syntax and builtins. See limits below. |
| Three.js / WebGL | **Not supported** — no WebGL/WebGPU context, so Three.js demos cannot run. Static threejs.org pages render. |
| GUI window | **Compiled, not visually verified** — this build environment has no display server |
| Android | **Cross-check blocked** — the Rust target is installed, but there is no NDK/SDK here (`ring` needs `aarch64-linux-android-clang`) and the GUI shell's `minifb` backend is X11-only, so no APK can be produced in this environment |

## Quick start

```bash
cargo build --release

./target/release/veyra                       # open the browser window with the start page
./target/release/veyra https://example.com   # open a URL
```

### Command line

| Command | What it does |
| --- | --- |
| `veyra [url]` / `veyra ui [url]` | Open the window at a URL (or the start page) |
| `veyra shot <url> <out.png>` | Headless render to PNG (`--width N --height N --wait MS --scroll Y`) |
| `veyra dump <url>` | Print page info, console output, errors and visible text |
| `veyra eval <url> <js>` | Load a page, run JavaScript in it, print the result |
| `veyra boxes <url>` | Print the layout box tree and text runs |
| `veyra fetch <url>` | Fetch a URL and print status, headers and body |
| `veyra version` / `veyra help` | Version / usage |

`shot`, `dump`, `eval` and `boxes` are headless, which is how this project is developed
and verified without a display.

## What is implemented

### Networking (`veyra-net`)
- Own HTTP/1.1 client over `rustls` (TLS 1.2/1.3) with `webpki-roots`
- DNS resolution, connection reuse, chunked transfer decoding, gzip/deflate bodies
- Redirect chains, cookie jar with attributes, in-memory + on-disk response cache
- `file://` and `about:newtab` document loading
- 3 MB per-script cap, response size caps, timeouts

### HTML & DOM (`veyra-html`, `veyra-dom`)
- UTF-8 tokenizer (byte-safe on multibyte input), implied tags, raw-text elements,
  void elements, attribute parsing, comments and doctypes
- Tree construction with foster parenting basics, character references
- DOM nodes, `querySelector`/`querySelectorAll` with CSS selector matching,
  `classList`, `dataset`, `innerHTML`/`outerHTML`/`textContent`, attribute APIs
- `EventTarget` with capture/bubble phases, `click`, `input`, `submit`, `keydown`,
  `DOMContentLoaded`, `load`; real click dispatch from the shell

### CSS (`veyra-css`)
- Stylesheet parser with `@media`, `@supports`, `@import`, `@font-face`, `@keyframes`
  (parsed, not animated), escapes and nested blocks
- Selector engine: type/class/id/attribute/pseudo-classes (`:hover`, `:nth-child`,
  `:not`, `:is`, …), combinators, specificity sorting
- Cascade with `!important`, inline styles, user-agent stylesheet, inheritance
- Colors (`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`, `rgb()`, `rgba()`, `hsl()`, named),
  custom properties resolved lazily at computed-value time, `var()` fallbacks
- `calc()` with `+ - * /` and parentheses; `min()`/`max()`/`clamp()`
- Backgrounds: colors, linear/radial gradients, images, repeat/size/position/clip,
  and `background-clip: text` (gradient headline text), rendered through a glyph mask
- Box model, `border-radius` incl. elliptical, per-side borders, `box-shadow`,
  `opacity`, `overflow`, `object-fit`, `filter` (parsed subset), 2D transforms
- Flexbox (grow/shrink/basis, justify/align, wrap, gaps) and CSS grid
  (`grid-template-columns/rows`, `fr` units, gaps, auto placement)
- Media queries against viewport width/height/aspect ratio; `prefers-*` conservative
- Length units: `px`, `em`, `rem`, `%`, `vw`, `vh`, `vmin`, `vmax`, `pt`, `ch`, `ex`
- Conservative `@supports`: unsupported modern color functions are skipped so
  authors' hex fallbacks apply, instead of being mis-rendered

### Layout & paint (`veyra-layout`, `veyra-paint`)
- Block, inline, inline-block, flex and grid formatting contexts
- Text measurement and line breaking with embedded DejaVu fonts (fontdue)
- Intrinsic sizing for buttons/form controls including padding, border and text
- Software RGBA rasterizer: anti-aliased glyphs, rounded rectangles, gradients,
  images (PNG/JPEG/GIF/WebP/BMP/ICO via `image`), clipping, opacity layers
- Saturating coordinate math so pathological pages cannot panic the renderer

### JavaScript (`veyra-js`)
A hand-written lexer, Pratt parser and tree-walking interpreter:
- ES5.1 core: prototypes, closures, `this`, getters/setters, `try`/`catch`/`finally`,
  regular expressions (ECMAScript syntax via the `regress` library)
- ES2015+: `let`/`const`, arrow functions, classes with `extends`/`super`/static
  fields/private `#names`, template literals + tagged templates + `String.raw`,
  destructuring, spread/rest, default params, `for…of`, `Map`/`Set`/`WeakMap`/
  `WeakSet`, `Symbol`, `Proxy`, iterators and generators, `Promise` with
  microtask ordering, `async`/`await` syntax, optional chaining, nullish coalescing,
  `Array`/`Object`/`String`/`Number`/`Math`/`JSON`/`Date`/`RegExp` methods,
  typed arrays, `TextEncoder`/`TextDecoder`
- Host APIs: `document` (including `document.fonts`), element/style/class APIs,
  `window`, `localStorage`/`sessionStorage`, `fetch` (synchronous-then-promise),
  `XMLHttpRequest`, `Image`, `structuredClone`, `setTimeout`/`setInterval`,
  `requestAnimationFrame`, `MutationObserver`, `performance`, console, cookies
- Errors are reported with stack traces into `veyra dump`; a failing script does not
  abort the page

## Compatibility matrix (verified by rendering)

| Target | Result | Evidence |
| --- | --- | --- |
| Tailwind v2 compiled CDN stylesheet | Styled: gradient hero, cards, grid, buttons | `artifacts/tailwind-css-cdn.png` |
| Tailwind v4 precompiled CSS output | Styled; gradients require v4 class names (`bg-linear-to-r`) | `artifacts/tailwind4-css.png` |
| Tailwind Play CDN (`cdn.tailwindcss.com`) | **Styled** — the unpatched CDN JIT compiles the page's utilities at runtime; gradient text, cards, grid and buttons all render | `tests/fixtures/tailwind-play-cdn.html`, `artifacts/tailwind-play-cdn.png` |
| `https://www.google.com` (Chrome UA) | Google serves its modern page; scripts run and it renders, but the JS-driven layout is still incomplete (a few runtime errors remain, content positioning is wrong). The placeholder page was the old UA path. | `artifacts/google.png` |
| `https://tailwindcss.com` | Visibly styled (hero, nav, image grid); some overflow and code-block glyph garbling | `artifacts/site-tailwindcss3.png` |
| `https://threejs.org` static gallery | Renders well (images, gradients, layout) | `artifacts/threejs.png` |
| Three.js WebGL demos | **Do not run** — no WebGL context | — |
| `https://news.ycombinator.com` | Renders as styled table/list page | `artifacts/site-news_ycombinator_com.png` |
| `https://en.wikipedia.org` | Renders article layout and images | `artifacts/wikipedia.png`, `artifacts/wikipedia2.png` |
| MDN docs | Mostly blank: content is rendered by ES modules | `artifacts/site-developer_mozilla_org.png` |
| Veyra start page | Renders (cards, gradients, layout) | `artifacts/newtab5.png` |

The fixtures live in `tests/fixtures/`:
- `tailwind-compiled-cdn.html` — the supported Tailwind path (compiled CSS from the CDN)
- `tailwind-play-cdn.html` — the unsupported path, kept so the gap stays measurable

## Known limitations (explicit, not simulated)

- **WebGL / WebGPU**: not implemented. Three.js, Babylon.js, shader demos, WebGL maps
  and 3D product viewers cannot run. Veyra does not fake a WebGL context.
- **ES modules**: `<script type="module">`, `import`/`export`, dynamic `import()` and
  import maps are not executed. Sites whose content is entirely module-rendered (MDN,
  many React/Vite apps) show little or nothing.
- **WebAssembly**: not implemented.
- **Tailwind Play CDN**: workable for the v3 Play CDN, which now compiles at runtime
  (`artifacts/tailwind-play-cdn.png`). The v4 browser build is not verified, and the
  compiler is CPU-heavy: allow it a few seconds and a large `VEYRA_MAX_STEPS`.
- **Google and similarly script-heavy sites**: Veyra identifies as Chrome 131 for
  both the HTTP `User-Agent` and `navigator.userAgent`, so Google serves its modern
  page instead of the fallback. That page's scripts run, but its layout is driven by
  runtime measurement and advanced DOM APIs that are still incomplete, so the result
  is not pixel-faithful yet. Google's anti-bot pages may also appear.
- **Android**: `aarch64-linux-android` Rust std is installed and the engine crates are
  portable Rust with no OS GUI dependency, but this environment has no Android NDK or
  SDK, so the cross-build stops in `ring`'s build script looking for
  `aarch64-linux-android-clang`. The window shell uses `minifb`, which has no Android
  back-end; an APK would need a native-activity/EGL surface plus an Android SDK
  (aapt2/d8/apksigner) to package it. Neither exists here, so no APK is shipped and
  no Android runtime behaviour is claimed.
- **SVG**: `<svg>` elements are laid out but not rasterized.
- **CSS animation/transition**: parsed and exposed to the JS API, not animated.
- **`<canvas>` 2D**: no painting back-end.
- **Iframes, workers, service workers, IndexedDB, WebSocket**: not implemented.
- **Fonts**: bundled DejaVu faces plus system CJK fallbacks; webfont files are parsed
  but not downloaded as font resources.
- **Forms**: inputs, textareas, selects and buttons lay out and accept text; they do
  not submit to a server by themselves.
- **JS engine semantics** that differ from spec: generators and `async` functions are
  eagerly evaluated (generators produce arrays, async functions return resolved
  promises), no real suspension. No `eval`, no `with`, no tail calls.
- **Printing, extensions, downloads UI, PDF viewer, devtools window**: not implemented.
  Devtools are CLI-oriented (`dump`, `boxes`, `eval`).
- **Real-site rendering is best-effort**: complex production sites still show layout
  overflow, clipped text or garbled syntax-highlighted code. This is an honest
  limitation of the current CSS/paint coverage, not a hidden setting.

## Building

### Linux / macOS
```bash
cargo build --release          # target/release/veyra
cargo test --workspace         # unit + integration tests
```

Runtime window dependencies are the usual X11/Wayland libraries used by `minifb`
(`libxkbcommon`, X11). The headless commands need none of them.

### Windows

Option A — build on Windows:
```powershell
cargo build --release          # target\release\veyra.exe
```

Option B — cross-compile from Linux with MinGW-w64:
```bash
rustup target add x86_64-pc-windows-gnu
sudo apt install gcc-mingw-w64-x86-64-posix      # or extract the .debs without root
cargo build --release --target x86_64-pc-windows-gnu
# → target/x86_64-pc-windows-gnu/release/veyra.exe
```

Option B was used to produce `dist/veyra-windows-x86_64.exe` (PE32+ x86-64, 8.5 MB).
It is a cross-compiled artifact and **has not been launched on Windows** in this
environment, so treat first-run behaviour there as unverified.

## Layout

```
src/main.rs              CLI entry point (ui, shot, dump, eval, boxes, fetch)
crates/veyra-common/     colors, geometry, URL parsing, JSON
crates/veyra-net/        HTTP/1.1 client, TLS, cookies, cache, redirects
crates/veyra-html/       tokenizer and tree builder
crates/veyra-dom/        DOM tree, selectors, events
crates/veyra-css/        parser, cascade, computed style
crates/veyra-layout/     box tree, flex/grid, fonts, text layout
crates/veyra-paint/      software rasterizer, gradients, images
crates/veyra-js/         lexer, parser, interpreter, builtins
crates/veyra-engine/     page lifecycle, DOM/JS bindings, timers, fetch
crates/veyra-shell/      minifb window, tabs, bookmarks, history, shortcuts
tests/fixtures/          HTML fixtures used for manual/automatic checks
artifacts/               headless screenshots referenced above
dist/                    prebuilt Windows executable
```

## Third-party crates (infrastructure only)

`rustls`/`webpki-roots` (TLS and trust anchors), `flate2` (gzip), `regress`
(ECMAScript regex grammar), `fontdue` (glyph rasterization), `image`/`png`
(decode/encode), `minifb` (windowing and framebuffer presentation). None of these is
a browser engine or a JavaScript engine; all parsing, styling, layout, painting and
script execution are Veyra's own code.

Licensed MIT.
