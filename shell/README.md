# shell/

Today screen, Start menu, task switcher, notifications and SIP keyboard: the modern take on the Windows Mobile shell. Layer: Apps.

The Today test image (`rp-today`) bakes in its wallpaper and its weather pictures from
`local/` (never committed). Make the pictures with
`python -I tools/shell-assets/icons.py assets/weather/vclouds local/icons/icons.rs`
before building; without them the screen draws outlines where they go. The icons are
VClouds' (credit and terms in `assets/weather/vclouds/README.md`).
