# Built-in patterns

Every `.rle` file here is embedded in the binary as a built-in pattern by
`build.rs`, and listed in the palette in file-name order. Each was written out
in this repository; the `#O` lines credit the people who found the patterns.

Add a pattern only if it can ship under gol-bevy's MIT licence: write small,
well-known patterns out by hand rather than copying files from another
program's pattern library, whose terms may differ. A new pattern also needs a
row in the built-in patterns table in `docs/patterns.qd`, which a test checks.
