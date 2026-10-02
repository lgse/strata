# SPDX-License-Identifier: MIT
"""Generic external providers: live menus, literal selection and fail-closed UI."""
import base64
import json
from pathlib import Path
import sys
import time

import pytest

# A trusted registration fixture, not a program discovered in the browsed folder.
PROGRAM = r'''
import json,sys,threading,time
from pathlib import Path
root,state,record=map(Path,sys.argv[1:])
lock=threading.Lock()
def emit(value):
 with lock:
  print(json.dumps(value),flush=True)
def watch():
 previous=None
 while True:
  try: now=(state.read_text(), (state.parent/'event').read_text() if (state.parent/'event').exists() else '')
  except OSError: now='offline'
  if now!=previous:
   previous=now;emit({'version':1,'event':'invalidate'})
  time.sleep(.1)
threading.Thread(target=watch,daemon=True).start()
for line in sys.stdin:
 r=json.loads(line);paths=r['paths'];mode=state.read_text()
 if (state.parent/'delay').exists(): time.sleep(.3)
 eligible=mode!='offline' and (r['method']=='query' or all(Path(p).is_relative_to(root) and Path(p).name!='local-only.txt' for p in paths))
 result={'version':1,'id':r['id']}
 if eligible:
  if r['method']=='query':
   result['decorations']=[{'path':p,'badge':'badge' if mode=='kept' else None,'description':'Available offline' if mode=='kept' else ''} for p in paths if Path(p).is_relative_to(root) and Path(p).name!='local-only.txt']
  elif r['method']=='menu':
   result['actions']=[{'id':'release' if mode=='kept' else 'keep','label':'Release test pin' if mode=='kept' else 'Keep test pin','icon':'badge'}]
  elif r['method']=='activate':
   record.write_text(json.dumps(r));state.write_text('kept');result['message']='Test pin accepted'
 emit(result)
'''

@pytest.fixture(autouse=True)
def provider_registration(test_environment, fixture_tree):
    folder = test_environment.config_home / "strata/providers/example"
    folder.mkdir(parents=True, mode=0o700)
    state = folder / "state"
    state.write_text("on-demand")
    script = folder / "provider.py"
    script.write_text(PROGRAM)
    # Small PNG, bounded and loaded only from this trusted registration.
    (folder / "badge.png").write_bytes(base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAHUlEQVR4nGM0qnj2n4ECwESJ5lEDRg0YNWAwGQAAosYCrxCcv1sAAAAASUVORK5CYII="))
    (folder / "provider.json").write_text(json.dumps({"version":1,"id":"example","command":[sys.executable,str(script),str(fixture_tree.root),str(state),str(folder/"record")],"icons":{"badge":"badge.png"}}))
    fixture_tree.path("local-only.txt").write_text("unrelated")
    return folder


def test_provider_updates_open_menu_and_revalidates_mixed_selection(strata, provider_registration):
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Keep test pin" in strata.menu_items(), "asynchronous provider menu")
    (provider_registration / "state").write_text("kept")
    strata.wait(lambda: strata.window.find(role="image", description="Available offline"), "badge accessibility after state event")
    strata.wait(lambda: "Release test pin" in strata.menu_items(), "event refresh of open menu")
    assert "Keep test pin" not in strata.menu_items()
    (provider_registration / "state").write_text("offline")
    strata.wait(lambda: "Release test pin" not in strata.menu_items(), "provider withdrawal")
    strata.wait(lambda: not strata.window.find(role="image", description="Available offline"), "badges withdrawn")
    strata.dismiss_menu()
    (provider_registration / "state").write_text("on-demand")
    strata.select_entry("todo.txt")
    strata.click_entry_with("local-only.txt", ["ctrl"])
    strata.open_context_menu("local-only.txt")
    assert "Keep test pin" not in strata.menu_items()
    strata.dismiss_menu()
    strata.select_entry("todo.txt")
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Keep test pin" in strata.menu_items(), "eligible selection")
    strata.choose_menu_item("Keep test pin")
    record = provider_registration / "record"
    strata.wait(record.exists, "provider action dispatched")
    assert json.loads(record.read_text())["paths"] == [str(strata.fixture.path("todo.txt"))]


@pytest.mark.preferences(browser_mode="columns")
def test_provider_background_receives_only_clicked_folder(strata, provider_registration):
    root = strata.fixture.root.name
    strata.pointer.click(
        strata.pane(root), at=strata.background_point(root), button=3
    )
    strata.wait(strata.context_menu, "background menu")
    strata.wait(lambda: "Keep test pin" in strata.menu_items(), "background provider action")
    strata.choose_menu_item("Keep test pin")
    record = provider_registration / "record"
    strata.wait(record.exists, "background action dispatched")
    request = json.loads(record.read_text())
    assert request["background"] is True
    assert request["paths"] == [str(strata.fixture.root)]


@pytest.mark.parametrize("invalidate", [False, True])
def test_refresh_keeps_unchanged_badges_and_open_menu_visible(strata, provider_registration, invalidate):
    (provider_registration / "state").write_text("kept")
    (provider_registration / "delay").touch()
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Release test pin" in strata.menu_items(), "initial provider menu")
    strata.wait(lambda: strata.window.find(role="image", description="Available offline"), "initial badge")
    if invalidate:
        (provider_registration / "event").write_text("refresh without changing state")
    deadline = time.monotonic() + 7
    while time.monotonic() < deadline:
        assert "Release test pin" in strata.menu_items(), "refresh withdrew an unchanged menu"
        assert strata.window.find(role="image", description="Available offline"), "refresh withdrew an unchanged badge"
        time.sleep(.04)
