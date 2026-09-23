# SPDX-License-Identifier: MIT

from __future__ import annotations

import base64

import pytest

from harness.fixtures import FixtureTree


# ZIP and header-encrypted 7z containing protected.txt; password: fixture-password.
ENCRYPTED_ARCHIVES = {
    "zip": "UEsDBAoACQAAABdsM12zQ1U4JwAAABsAAAANABwAcHJvdGVjdGVkLnR4dFVUCQADXuOual7jrmp1eAsAAQToAwAABOgDAACQyEkvHVKZmURUaFrAXIjmRah+W9tCT9En6fCshzKr4S/pzBgPWBlQSwcIs0NVOCcAAAAbAAAAUEsBAh4DCgAJAAAAF2wzXbNDVTgnAAAAGwAAAA0AGAAAAAAAAQAAAKSBAAAAAHByb3RlY3RlZC50eHRVVAUAA17jrmp1eAsAAQToAwAABOgDAABQSwUGAAAAAAEAAQBTAAAAfgAAAAAA",
    "7z": "N3q8ryccAARK90rXoAAAAAAAAAAvAAAAAAAAAM2lRMsbe4LgOwc6BQCZYKv5bMKO/JVT8lQovoXQhOLoYQdQk+uWUC0i3pVYfHvs3WsWQxoB/iug9vZINEehfZwB2T+4IWJod2G0L8SsGPd8fL2S7eqrf0iiMWv3YxcjHoairfwd3pFxFkmBBuIT/XLKPJC7MVu79tKCB+pPvxRxvfdd0S2wr+A0/f/dCbpioH0eSSubzTPZlYFEs1FLcbo7KwIJFwYgAQmAgAAHCwEAASQG8QcBElMPiTvkREbaQxsUI9zKFIworgxyCgEqHRUKAAA=",
}


@pytest.fixture
def fixture_tree(request):
    extension = request.param
    tree = FixtureTree.create({f"encrypted.{extension}": ""})
    tree.path(f"encrypted.{extension}").write_bytes(base64.b64decode(ENCRYPTED_ARCHIVES[extension]))
    try:
        yield tree
    finally:
        tree.cleanup()


@pytest.mark.parametrize("fixture_tree", ["zip", "7z"], indirect=True)
def test_archive_password_retry_crosses_the_real_sandbox(strata, fixture_tree):
    filename = next(fixture_tree.root.iterdir()).name
    strata.select_entry(filename)
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("Password-protected archive"), "the password prompt")
    assert not strata.preview_shows("protected.txt")

    strata.wait(
        lambda: strata.preview().find(states={"editable", "focused"}),
        "the password entry to take focus",
    )
    strata.keyboard.type_text("incorrect-password")
    strata.keyboard.press("Return")
    strata.wait(lambda: strata.preview_shows("The password is incorrect."), "a rejected password")
    assert not strata.preview_shows("protected.txt")

    strata.wait(
        lambda: strata.preview().find(states={"editable", "focused"}),
        "the cleared password entry to take focus",
    )
    strata.keyboard.type_text("fixture-password")
    strata.pointer.click(strata.preview().find(role="button", name="Unlock"))
    strata.wait(lambda: strata.preview_shows("protected.txt"), "the unlocked archive listing")
    assert not strata.preview_shows("synthetic archive contents")
    assert sorted(path.name for path in fixture_tree.root.iterdir()) == [filename]
    strata.pointer.click(strata.preview().find(role="button", name="Close preview (Space)"))
    strata.wait(lambda: strata.preview() is None, "the preview to close")
