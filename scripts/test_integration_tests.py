# SPDX-License-Identifier: MIT

from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import integration_tests  # noqa: E402

MANIFEST = 'schema_version = 1\nid = "{id}"\nname = "X"\ndescription = "Y"\ncategory = "other"\nmaintainer = "octocat"\nruntime = "{runtime}"\nentry = "provider.py"\n'


def integration(root: Path, id: str, *, passing: bool, runtime: str = sys.executable) -> Path:
    directory = root / id
    (directory / "tests").mkdir(parents=True)
    (directory / "catalog.toml").write_text(MANIFEST.format(id=id, runtime=runtime))
    (directory / "provider.py").write_text("print('hi')\n")
    outcome = "pass" if passing else "self.fail('broken')"
    (directory / "tests" / "test_provider.py").write_text(
        f"import unittest\n\nclass T(unittest.TestCase):\n    def test_it(self):\n        {outcome}\n"
    )
    return directory


class IntegrationTestsTest(unittest.TestCase):
    def test_discovers_integrations_and_reports_failures(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            integration(root, "alpha", passing=True)
            (root / "alpha" / "tests" / "apt-packages.txt").write_text("python3-gi\n# comment\ndbus-daemon\n")
            integration(root, "_template", passing=False)
            (root / ".hidden").mkdir()
            self.assertEqual(
                [path.name for path in integration_tests.integration_directories(root)], ["alpha"]
            )
            self.assertEqual(integration_tests.apt_packages(root), ["python3-gi", "dbus-daemon"])
            self.assertEqual(integration_tests.main(["--root", str(root)]), 0)
            integration(root, "beta", passing=False)
            self.assertEqual(integration_tests.main(["--root", str(root)]), 1)

    def test_an_integration_without_tests_only_needs_to_compile(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = integration(root, "gamma", passing=True)
            (directory / "tests" / "test_provider.py").unlink()
            (directory / "tests").rmdir()
            self.assertEqual(integration_tests.main(["--root", str(root)]), 0)
            (directory / "provider.py").write_text("def broken(:\n")
            self.assertEqual(integration_tests.main(["--root", str(root)]), 1)

    def test_an_empty_catalog_passes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            self.assertEqual(integration_tests.main(["--root", str(Path(temporary) / "none")]), 0)


if __name__ == "__main__":
    unittest.main()
