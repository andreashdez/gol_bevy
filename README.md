# Conway's Game of Life

An implementation of Conway's Game of Life with a native desktop GUI built on Bevy.

## Controls

```text
Controls

Space: run/pause
j: step
r: randomize
x: clear
0: reset epoch
+ / -: speed
g: select glider stamp
b: select blinker stamp
l: select LWSS stamp
z: rotate stamp
f: flip stamp horizontally
c: clear active stamp
m: toggle boundary mode
F3: toggle diagnostics
t: toggle grid
/: toggle help
Wheel: zoom camera
WASD / Arrows: pan camera
v: reset camera
Left drag: draw
Left click (stamp): place pattern
Right drag: erase
q / Esc: quit
```

## RLE patterns

The project supports parsing classic Life RLE strings (`x = <w>, y = <h>`) with:

- comment lines starting with `#`
- run counts like `3o`, `10b`, `2$`
- row breaks (`$`) and terminator (`!`)

Example:

```text
x = 8, y = 4
8o$8o$8o$8o!
```
