# remkrom - reimplementation of genkfs in Rust

Writes individual files into a ROM image.  Works as a drop-in
replacement for [genkfs](https://github.com/KnightOS/genkfs/).  It has
been tested and has the same behavior as the C version.

## Note on undefined behavior
The original C version had undefined behavior in a few places,
especially with regard to arithmetic.  The rewrite in Rust contains no
undefined behavior, but has a compile-time flag to replicate what C
does (as least on my x86-64 machine), wrapping arithmetic.

```sh
$ cargo build --release --features "c-undef"
```

## Usage
```
regenkfs 0.1.0
A reimplementation of the KnightOS genkfs tool in Rust

USAGE:
    regenkfs <input> <model>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

ARGS:
    <input>    The ROM file to write the filesystem to
    <model>    Path to a directory that will be copied into / on the new filesystem
```
