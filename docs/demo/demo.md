# The terminal demo

`demo.svg`, the animation at the top of the [repository README](../..), is a
recording of the five `sf-impact-*` commands made with
[asciinema](https://asciinema.org/) and rendered to SVG with
[svg-term-cli](https://github.com/marionebl/svg-term-cli).

| File | What it is |
|---|---|
| `demo.sh` | The script that is recorded. It builds nothing: it copies `fixtures/project` to `/work`, makes it a git repository with a base commit on `main` and one commit on a branch that changes an Apex class, then types out and runs the five released commands from `PATH`. The identity and dates of the commits are fixed, so the commit ids on screen are the same every time. |
| `demo.cast` | The asciinema recording (asciicast v2, plain text). Its header has the terminal size, `TERM` and the idle limit, and no timestamp. |
| `demo.svg` | The animation rendered from `demo.cast`: plain SVG with CSS animation, which plays inline on GitHub. |
| `regenerate.sh` | Records and renders, in containers. |
| `requirements.txt`, `package.json`, `package-lock.json` | The pinned recorder and renderer: asciinema 2.4.0 (with its checksum) and svg-term-cli 2.1.1 (with the lockfile's integrity hashes). The base images are pinned by digest in `regenerate.sh`. |

The output on screen is what the commands wrote; nothing is typed in by hand
and nothing is edited afterwards.

## Regenerate

From the repository root, with the five commands built in release mode
(`cargo build --release --workspace --bins --locked --target x86_64-unknown-linux-musl`),
and podman (or `CONTAINER=docker`) and network access:

```sh
docs/demo/regenerate.sh target/x86_64-unknown-linux-musl/release
```

That records `demo.cast` from the commands in that directory and renders
`demo.svg`. To change only the look, render again from the committed
recording, which runs none of the commands:

```sh
docs/demo/regenerate.sh --render
```

Edit `demo.sh` to change what is shown. The terminal is 112 columns by 30 rows
(`COLS` and `ROWS` in `regenerate.sh`); keep `demo.svg` under about 300 KB.
