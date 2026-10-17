# SPDX-License-Identifier: MIT
import json
from pathlib import Path
import stat
import tempfile
import unittest

from install import install


class InstallationTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.config = self.root / "config"
        self.state = self.root / "state"
        self.provider = self.config / "strata/providers/kdeconnect"

    def test_opt_in_preserves_settings_actions_and_other_providers(self):
        existing = ["settings.toml", "actions/custom/action.toml", "providers/other/provider.json"]
        for name in existing:
            path = self.config / "strata" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("preserve")
        changed, backups = install(self.config, self.state)
        self.assertEqual(len(changed), 3)
        self.assertIsNone(backups)
        for name in existing:
            self.assertEqual((self.config / "strata" / name).read_text(), "preserve")
        manifest = json.loads((self.provider / "provider.json").read_text())
        self.assertEqual(manifest["command"], ["/usr/bin/python3", str(self.provider / "provider.py")])
        for name in ("provider.py", "messages.json"):
            self.assertEqual((self.provider / name).read_bytes(), Path(__file__).with_name(name).read_bytes())
        self.assertEqual(stat.S_IMODE(self.provider.stat().st_mode), 0o700)
        for path in changed:
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)

    def test_reinstall_is_idempotent_and_updates_keep_private_backups(self):
        install(self.config, self.state)
        self.assertEqual(install(self.config, self.state), ([], None))
        script = self.provider / "provider.py"
        script.write_text("old provider")
        changed, backups = install(self.config, self.state)
        self.assertEqual(changed, [script])
        self.assertEqual((backups / "provider.py").read_text(), "old provider")
        self.assertEqual(stat.S_IMODE((backups / "provider.py").stat().st_mode), 0o600)

    def test_symlink_destinations_are_refused_before_replacement(self):
        self.provider.mkdir(parents=True)
        target = self.root / "unrelated"
        target.write_text("keep")
        (self.provider / "provider.json").symlink_to(target)
        with self.assertRaises(ValueError):
            install(self.config, self.state)
        self.assertEqual(target.read_text(), "keep")
        self.assertFalse((self.provider / "provider.py").exists())


if __name__ == "__main__":
    unittest.main()
