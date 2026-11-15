#!/usr/bin/env python3
"""Write THIRD-PARTY-NOTICES.md: every crate gevrek is built with, its
version, its licence and where it comes from, as `cargo metadata` tells it.

    python scripts/third-party-notices.py

Run it again when Cargo.lock changes."""
import collections
import json
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
told = subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=ROOT,
                      capture_output=True, text=True, check=True, encoding="utf-8").stdout
data = json.loads(told)
own = set(data["workspace_members"])
crates = sorted((p for p in data["packages"] if p["id"] not in own), key=lambda p: (p["name"].lower(), p["version"]))
by_licence = collections.Counter(p.get("license") or "see its repository" for p in crates)

HEAD = """# Third-party notices

gevrek is published under the MIT licence (see `LICENSE`). It is built with
the work of many others, listed here with the licence each is used under.
Where a crate offers a choice of licences, gevrek uses it under the MIT
licence where that is offered, and otherwise under the Apache-2.0 licence.

## What gevrek stands on

| | What it is to gevrek | Licence |
|---|---|---|
| [Smithay](https://github.com/Smithay/smithay) | The Wayland compositor library under gevrek's Linux backend, and its client toolkit under the shell's surfaces | MIT |
| [egui and eframe](https://github.com/emilk/egui) | Everything gevrek draws: the studio, the panel, the launcher, the control centre, the lock screen | MIT OR Apache-2.0 |
| [wgpu](https://github.com/gfx-rs/wgpu) | The graphics device egui draws with | MIT OR Apache-2.0 |
| [windows-rs](https://github.com/microsoft/windows-rs) | The Windows API, for the Windows backend | MIT OR Apache-2.0 |
| [tonic](https://github.com/hyperium/tonic) and [prost](https://github.com/tokio-rs/prost) | The line between gevrek's programs | MIT, Apache-2.0 |
| [Phosphor Icons](https://phosphoricons.com), through [egui-phosphor](https://github.com/amPerl/egui-phosphor) | The icons of gevrek's programs | MIT |

## Fonts in the programs

gevrek's programs carry the fonts that egui carries
(`epaint_default_fonts`): **Ubuntu Light** (Ubuntu Font Licence 1.0),
**Hack** (MIT), **Noto Emoji** (SIL Open Font License 1.1) and
**emoji-icon-font** (MIT). Where the system has a font of its own for the
interface, gevrek reads that from the system and carries no copy of it.

## Its own marks

The gevrek logo, icons and boot pictures under `assets/` and `resources/`
are gevrek's own and are published with it under the MIT licence.

## Every crate

"""

with open(os.path.join(ROOT, "THIRD-PARTY-NOTICES.md"), "w", encoding="utf-8", newline="\n") as out:
    out.write(HEAD)
    out.write(f"{len(crates)} crates, as Cargo.lock has them. Not every one is in every program: some are\n"
              "for one system only, some only build or test gevrek.\n\n")
    out.write("| Licence | Crates |\n|---|---|\n")
    for licence, count in by_licence.most_common():
        out.write(f"| {licence} | {count} |\n")
    out.write("\n| Crate | Version | Licence | Source |\n|---|---|---|---|\n")
    for p in crates:
        source = p.get("repository") or p.get("homepage") or f"https://crates.io/crates/{p['name']}"
        out.write(f"| {p['name']} | {p['version']} | {p.get('license') or 'see its repository'} | <{source}> |\n")
print(f"THIRD-PARTY-NOTICES.md: {len(crates)} crates")
