#!/usr/bin/env python3
"""Embed the shared rust-i18n installer catalog in the standalone Bash installer."""

import argparse
import json
import pathlib
import shlex

ROOT = pathlib.Path(__file__).resolve().parents[1]
START = "# installer-translations:start"
END = "# installer-translations:end"


def render(catalog: dict) -> str:
    messages = {key.removeprefix("installer."): value for key, value in catalog.items() if key != "_version"}
    locales = list(next(iter(messages.values())))
    lines = [
        START,
        "# Generated from data/locales/installer.json by scripts/installer_messages.py.",
        "declare -A INSTALLER_MESSAGES=()",
        'case "$(installer_locale)" in',
    ]
    for locale in locales:
        lines.append(f"  {locale}) INSTALLER_MESSAGES=(")
        for key, translations in messages.items():
            if set(translations) != set(locales):
                raise ValueError(f"Inconsistent locales for {key}")
            lines.append(f"    [{key}]={shlex.quote(translations[locale])}")
        lines.append("  ) ;;")
    lines.extend(["esac", END])
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="Update install.sh instead of checking it")
    args = parser.parse_args()
    installer = ROOT / "install.sh"
    source = installer.read_text(encoding="utf-8")
    catalog = json.loads((ROOT / "data/locales/installer.json").read_text(encoding="utf-8"))
    before, remainder = source.split(START, 1)
    _, after = remainder.split(END, 1)
    updated = before + render(catalog) + after
    if args.write:
        installer.write_text(updated, encoding="utf-8")
    elif source != updated:
        parser.exit(1, "Installer translations are stale; run python3 scripts/installer_messages.py --write\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
