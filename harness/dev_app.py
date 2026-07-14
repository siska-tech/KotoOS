"""Editor-facing app helper (KOTO-0190).

Resolves "the app the edited file belongs to" from ``apps/apps.json`` so
VS Code tasks (``.vscode/tasks.json``) stay zero-config: any file under
``apps/<dir>/...`` (source, include, sprite, audio, scenario) maps to the
registered app whose ``source`` lives in the same ``apps/<dir>``.

Commands (paths may be absolute; they are made repo-relative):

    python harness/dev_app.py run FILE          rebuild apps, headless run + --inspect
    python harness/dev_app.py screenshot FILE   rebuild apps, capture a frame BMP
    python harness/dev_app.py watch FILE        windowed live-reload loop (KOTO-0191)

``screenshot`` writes to ``target/koto-dev/<app_dir>.bmp`` (git-ignored) and,
when ``apps/<dir>/scenarios/frame_capture.txt`` exists, drives the app with it
so the captured frame is gameplay rather than the cleared exit frame (see the
App Development Loop guide).

``watch`` launches ``koto-sim --window --app <id> --watch apps/<dir>``: saving
any file under the app rebuilds it and relaunches it in the same window. When
``apps/<dir>/scenarios/watch_replay.txt`` exists it is passed as
``--watch-replay`` so every relaunch replays back to the scene under iteration.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def resolve_app(file_arg: str) -> tuple[str, str, str]:
    """Map an edited file to (app_id, app-folder-relposix, package).

    Walks up from the edited file to the nearest ``app.json`` (KOTO-0195), so
    any file in an app tree — source, include, sprite, audio, scenario —
    resolves to its owning app.
    """
    path = Path(file_arg)
    abs_path = path if path.is_absolute() else (ROOT / path)
    abs_path = abs_path.resolve()
    try:
        abs_path.relative_to(ROOT)
    except ValueError:
        sys.exit(f"dev_app: {file_arg} is outside the repository")
    for folder in [abs_path, *abs_path.parents]:
        descriptor = folder / "app.json"
        if descriptor.is_file():
            app = json.loads(descriptor.read_text(encoding="utf-8"))
            rel = folder.relative_to(ROOT).as_posix()
            return app["app_id"], rel, app.get("package", folder.name)
    sys.exit(f"dev_app: no app.json found above {path.as_posix()}")


def run(command: list[str]) -> None:
    completed = subprocess.run(command, cwd=ROOT)
    if completed.returncode != 0:
        sys.exit(completed.returncode)


def build_apps() -> None:
    run([sys.executable, str(ROOT / "harness" / "build_apps.py")])


def main() -> None:
    if len(sys.argv) != 3 or sys.argv[1] not in ("run", "screenshot", "watch"):
        sys.exit(__doc__)
    command, file_arg = sys.argv[1], sys.argv[2]
    app_id, app_rel, package = resolve_app(file_arg)
    if command == "watch":
        sim = [
            "cargo", "run", "-q", "-p", "koto-sim", "--features", "window", "--",
            "--window", "--app", app_id, "--watch", app_rel,
        ]
        replay = ROOT / app_rel / "scenarios" / "watch_replay.txt"
        if replay.exists():
            sim += ["--watch-replay", str(replay.relative_to(ROOT).as_posix())]
        run(sim)
        return
    build_apps()
    if command == "run":
        run(["cargo", "run", "-q", "-p", "koto-sim", "--", "--app", app_id, "--inspect"])
        return
    out_dir = ROOT / "target" / "koto-dev"
    out_dir.mkdir(parents=True, exist_ok=True)
    image = out_dir / f"{package}.bmp"
    sim = ["cargo", "run", "-q", "-p", "koto-sim", "--", "--app", app_id]
    scenario = ROOT / app_rel / "scenarios" / "frame_capture.txt"
    if scenario.exists():
        sim += ["--app-script", str(scenario.relative_to(ROOT).as_posix())]
    sim += ["--image", str(image)]
    run(sim)
    print(f"screenshot: {image}")


if __name__ == "__main__":
    main()
