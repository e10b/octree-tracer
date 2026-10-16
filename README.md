# An octree raytracer

This time i'm focusing on huge worlds by streaming data to the GPU. This project is a follow on from [voxel-tracing-playground](https://github.com/Ria8651/voxel-tracing-playground) and [df-tracer](https://github.com/Ria8651/df-tracer).

## Running

This project is very easy to run.

1. [Install rust](https://www.rust-lang.org/tools/install)
2. Clone this repository
3. Run `cargo run --release` in this repo's directory

### Optional authored brightmaps

For authored Doom texture and flat emission, clone
[Brightmaps Plus](https://github.com/NightFright2k19/gzdoom_brightmaps) into
`files/gzdoom_brightmaps`, or set `DOOM_BRIGHTMAPS_DIR` to its checkout. The
artwork is loaded at runtime and is intentionally not vendored in this repository.
Only textures covered by an authored mask emit light.

Path-traced frames are denoised with SVGF. Temporal reprojection accumulates
demodulated illumination together with its first and second luminance moments.
A 7×7 depth- and normal-weighted pool of those moments supplies the variance
that drives five à-trous wavelet passes at strides 1, 2, 4, 8 and 16. The
filtered illumination is remodulated by the center pixel albedo. Path samples
use a vendored 64×64 void-and-cluster blue-noise texture from
[Calinou/free-blue-noise-textures](https://github.com/Calinou/free-blue-noise-textures),
distributed under CC0 (see `assets/blue-noise/COPYING.txt`).

If you have any problems feel free to create an issue.

## [Data layout](LAYOUT.md)

## Screenshots

https://user-images.githubusercontent.com/66388895/155472265-6cc15bf9-8520-461a-99b4-a9a6b9b9a512.mov

<img width="1413" alt="Island" src="https://user-images.githubusercontent.com/66388895/155472467-2fb6606c-471b-43be-8193-3ebea31a69f8.png">

![dragon](https://user-images.githubusercontent.com/66388895/154792269-f3662b15-85b1-4451-bf07-a92ab676d298.png)

![fractal](https://user-images.githubusercontent.com/66388895/154792107-4b2a52bd-63b4-4170-8b0a-01846d3cb597.png)
