# Shields resources

`ubo-scriptlets.json` holds [uBlock Origin](https://github.com/gorhill/uBlock)'s
scriptlets (`set-constant`, `json-prune`, `trusted-*`, …) converted to the
format adblock-rust's `Engine::use_resources` takes. uBlock Origin's filter
lists reference these scriptlets for things like YouTube video ads, and
Brave's `adblock-resources` (the other file Shields downloads) doesn't include
them.

The app downloads this file at runtime (see `UBO_RESOURCES_URL` in
`src-tauri/src/shields.rs`) and caches it; it is not compiled into Serica.

**License:** the contents are uBlock Origin code, licensed under the
[GNU GPL v3](https://github.com/gorhill/uBlock/blob/master/LICENSE.txt),
not the MIT license that covers the rest of this repository.

## Regenerating

Download a uBlock Origin release and run the converter (Node 20+):

```bash
curl -L https://github.com/gorhill/uBlock/archive/refs/tags/<version>.tar.gz | tar -xz
node rust/tools/build-ubo-scriptlets.mjs uBlock-<version>/src/js/resources rust/resources/ubo-scriptlets.json <version>
```

Commit the result; running browsers pick it up on their next list update
(every 4 days, or Shields → update lists).
