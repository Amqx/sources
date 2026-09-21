"""Small presentation helpers shared by command implementations."""

from __future__ import annotations

import sys
from typing import NoReturn


def die(message: str) -> NoReturn:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def info(message: str) -> None:
    print(message, flush=True)
