# wetink

`tail -f` for Markdown. A terminal viewer that watches a Markdown file and
highlights the blocks that change, made for following documents that coding
agents write. Changed blocks flash, then keep a gutter mark: `▎` green for
added, `▎` amber for modified, and `┄┄ N blocks removed ┄┄` where blocks
were removed. Press `d` to see a diff against the version you opened.

![wetink following a file as an agent edits it](demo/wetink.gif)

## Install

```sh
brew install rooa/tap/wetink
```

Or download a binary for macOS, Linux or Windows from
[Releases](https://github.com/rooa/wetink/releases), or build it with Cargo:

```sh
cargo install --git https://github.com/rooa/wetink
```

## Usage

```sh
wetink NOTES.md            # watch and view (the file may not exist yet)
wetink --print NOTES.md    # render once to stdout
wetink -w 80 NOTES.md      # cap content width (default 100)
wetink -t light NOTES.md   # theme: auto (default), dark or light
wetink --mouse NOTES.md    # click a change to show its diff
```

| Key | Action |
|---|---|
| `j`/`k`, `↓`/`↑` | scroll |
| `space`/`b`, `Ctrl-d`/`Ctrl-u` | page / half page |
| `g`/`G` | top / bottom |
| `n`/`N` | select the next / previous change |
| `d` | show / hide the diff of the selected change |
| `c` | accept the current version (clear marks) |
| `f` | toggle follow (scroll to new changes) |
| `q` | quit |

To try it, run `wetink demo.md` in one pane and
`python3 scripts/simulate.py demo.md -d 1` in another. `demo/record.sh`
re-records the GIF above (needs tmux, asciinema and agg).

## Performance

Rendering once to stdout, compared with glow 2.1.2, on an Apple Silicon Mac
(median time / peak memory; times include about 12 ms of process start):

| | binary | 18 KB doc | 184 KB doc | 920 KB doc |
|---|---|---|---|---|
| glow | 16.5 MB | 292 ms / 40 MB | 2.53 s / 58 MB | 12.5 s / 141 MB |
| wetink | 2.7 MB | 27 ms / 13 MB | 32 ms / 18 MB | 68 ms / 41 MB |
| wetink, no syntax highlighting | 1.1 MB | 13 ms / 3 MB | 21 ms / 8 MB | 54 ms / 26 MB |

While watching, one edit to the 184 KB document takes about 5 ms to
re-render, and an idle viewer uses no measurable CPU. Build without syntax
highlighting with `--no-default-features`.

## License

MIT
