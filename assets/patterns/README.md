# Built-in patterns

Every `.rle` file here is embedded in the binary as a built-in pattern by
`build.rs`. The small ones (Acorn, Blinker, Die Easy, Diehard, Glider, Gosper
glider gun, LWSS, Pentadecathlon, Pulsar, R-pentomino) were written out in this
repository; their `#O` lines credit the people who found the patterns.

The two large collections come from the pattern library distributed with
[Golly](https://golly.sourceforge.io), in `Patterns/Life/Miscellaneous/` of its
[source repository](https://sourceforge.net/p/golly/code/ci/master/tree/Patterns/Life/Miscellaneous/)
(checked against commit `53bdb74`, November 2026):

| File | Source | Changes |
|---|---|---|
| `cambrian-explosion.rle` | Golly's `cambrian-explosion.rle`: based on Alan Hensel's `a-plus.lif`, with patterns from 2004–2006 selected from Koenig's Life News weblog | The `#N` line names the pattern rather than the file |
| `ordovician-radiation.rle` | Golly's `ordovician-radiation.rle`, curated by Matthias Merzenich and first posted to the [ConwayLife forums](https://conwaylife.com/forums/viewtopic.php?p=114098#p114098) in 2020 | None |

Golly is distributed under the GNU General Public License, version 2 or later.
Its pattern files carry no licence of their own, while gol-bevy is under the
MIT licence; whether the GPL covers pattern data distributed with Golly has not
been settled for this project. Credits for the individual patterns in each
collection are in the files' `#C` comments.
