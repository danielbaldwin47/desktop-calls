"""The Mesa shader cache every scenario of a run shares, filled first, as on a
Desktop that has run the app before (#298).

Each scenario's own cache starts empty. On a machine with no GPU, a surface
whose first frame the app had no quiet moment to prepare then compiles its
shader programs on the main loop, which held it 0.6 to 1.4 s in CI and
failed Banner and Handoff bounds that a user's warm Desktop meets. So the
scenario recipes (`just test`, `just ci-scenarios`, `just test-file`,
`just test-ci-like`) first run one scenario that lets the app prepare every
surface's first frame into this cache, then give the same cache to every
scenario through `SCENARIO_SHADER_CACHE`.

`SCENARIO_SHADER_CACHE` already set is kept as it is, and set but empty
means a cold cache for each scenario, as when timing first frames by hand.
`just ci-first-frame-timings` fills a cache of its own to judge them.
"""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys

import free_space

VARIABLE = "SCENARIO_SHADER_CACHE"
CACHE = pathlib.Path(__file__).resolve().parent.parent / "target" / "scenario-shader-cache"
WARM_UP_SCENARIO = (
    "shader_cache::the_app_shows_every_surface_once_into_the_shared_shader_cache"
)
WARM_UP = [
    "cargo", "test", "--locked", "--quiet", "-p", "test-harness", "--test", "scenarios", "--",
    "--ignored", "--exact", WARM_UP_SCENARIO,
]


class WarmUpFailed(Exception):
    """The warm-up scenario failed; the message says how to see why."""


def wanted(environment: dict[str, str]) -> bool:
    """Whether a run with this environment should warm the shared cache:
    not when the caller named a cache, or asked for none."""
    return VARIABLE not in environment


def warm(environment: dict[str, str], run=subprocess.run) -> dict[str, str]:
    """Fills the shared cache, unless the caller named one or asked for
    none, and returns the environment that gives it to every scenario.
    `run` stands in for running the warm-up in tests, and runs it confined
    for `just test-ci-like`, since Mesa keeps each renderer's programs
    apart. First it looks at free space on /tmp and /home, where scenarios
    keep their folders and cargo its builds, and raises free_space.Refused
    if either is nearly full (#397)."""
    free_space.preflight()
    if not wanted(environment):
        return environment
    CACHE.mkdir(parents=True, exist_ok=True)
    warmed = dict(environment, **{VARIABLE: str(CACHE)})
    print(f"Filling the scenarios' shared shader cache in {CACHE} (#298)", file=sys.stderr, flush=True)
    if run(WARM_UP, env=warmed).returncode != 0:
        raise WarmUpFailed(
            f"The warm-up scenario failed, so no scenario ran.\n"
            f"Next: {VARIABLE}={CACHE} cargo test -p test-harness --test scenarios -- "
            f"--ignored --exact {WARM_UP_SCENARIO} --nocapture"
        )
    return warmed


def main() -> int:
    """Runs the command in the arguments with the shared cache filled and
    named, as `just test` and `just ci-scenarios` do."""
    try:
        environment = warm(dict(os.environ))
    except (WarmUpFailed, free_space.Refused) as failure:
        print(failure, file=sys.stderr)
        return 1
    return subprocess.run(sys.argv[1:], env=environment).returncode


if __name__ == "__main__":
    sys.exit(main())
