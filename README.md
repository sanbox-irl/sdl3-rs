Rust-SDL2_ttf
===============

[![Build Status](https://travis-ci.org/andelf/rust-sdl2_ttf.svg?branch=master)](https://travis-ci.org/andelf/rust-sdl2_ttf)
[![Build Status](https://drone.io/github.com/andelf/rust-sdl2_ttf/status.png)](https://drone.io/github.com/andelf/rust-sdl2_ttf/latest)

Rust bindings for SDL2_ttf

Requirements
------------

* [Rust-sdl2](https://github.com/AngryLawyer/rust-sdl2)
* sdl_ttf 2.0 development libraries
* Rust master

Installation
------------

```
# compile your rust-sdl2 somewhere
git clone https://github.com/andelf/rust-sdl2_ttf
cd rust-sdl2_ttf
rustc -L. src/sdl2_ttf/lib.rs
# OR if you are using the mac framework version
rustc -L. --cfg mac_framework src/sdl2_ttf/lib.rs
```

Demo
----

```
rustc -L. src/demo/main.rs -o demo
./demo font.[ttf|ttc|fon]
```
