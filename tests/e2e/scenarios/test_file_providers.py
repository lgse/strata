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
import json,os,sys,threading,time
from pathlib import Path
root,state,record=map(Path,sys.argv[1:])
(state.parent/'pid').write_text(str(os.getpid()))
lock=threading.RLock()
generation=0
def emit(value):
 with lock:
  print(json.dumps(value),flush=True)
def watch():
 global generation
 previous=None
 while True:
  try: now=(state.read_text(), (state.parent/'event').read_text() if (state.parent/'event').exists() else '')
  except OSError: now='offline'
  if now!=previous or (state.parent/'storm').exists():
   previous=now
   with lock:
    generation+=1
    event={'version':1,'event':'invalidate'}
    if not (state.parent/'legacy').exists(): event.update(revision=generation,paths=[str(root)])
    emit(event)
  time.sleep(.1)
threading.Thread(target=watch,daemon=True).start()
for line in sys.stdin:
 r=json.loads(line);paths=r['paths']
 if (state.parent/'delay').exists(): time.sleep(.3)
 with lock:
  mode=state.read_text()
  eligible=mode!='offline' and (r['method']=='query' or all(Path(p).is_relative_to(root) and Path(p).name!='local-only.txt' for p in paths))
  result={'version':1,'id':r['id']}
  if r['method']=='query': result['decorations']=[]
  elif r['method']=='menu': result['actions']=[]
  if eligible:
   if r['method']=='query':
    result['decorations']=[{'path':p,'badge':'badge' if mode=='kept' else None,'description':'Available offline' if mode=='kept' else '', 'priority':int((state.parent/'priority').read_text()) if (state.parent/'priority').exists() else 0} for p in paths if Path(p).is_relative_to(root) and Path(p).name!='local-only.txt']
   elif r['method']=='menu':
    result['actions']=[{'id':'release' if mode=='kept' else 'keep','label':'Release test pin' if mode=='kept' else 'Keep test pin','icon':'badge'}]
    if (state.parent/'context').exists(): result['actions'][0]['context']=(state.parent/'context').read_text()
    if (state.parent/'tree').exists():
     result['actions']=[{'id':'inspect','label':'Inspect test state'}, {'id':'availability','label':'Offline availability','children':result['actions']+[{'id':'details','label':'Details','children':[{'id':'info','label':'Inspect nested state'}]}]}]
     if (state.parent/'relabel').exists():
      result['actions'][1]['label']='Local availability'
      result['actions'][1]['children'][0]['label']='Release updated pin' if mode=='kept' else 'Keep test pin'
     if (state.parent/'deep-relabel').exists():
      result['actions'][1]['label']='Cloud availability'
      result['actions'][1]['children'][1]['label']='Updated details'
      result['actions'][1]['children'][1]['children'][0]['label']='Inspect refreshed state'
     if (state.parent/'hide-inspect').exists(): result['actions'].pop(0)
   elif r['method']=='activate':
    record.write_text(json.dumps(r));state.write_text('kept');result['message']='Test pin accepted'
    if (state.parent/'disconnect').exists(): sys.exit(0)
    if (state.parent/'partial').exists():
     result.update(message='One test pin accepted; another was rejected',outcome={'status':'partial','accepted':1,'total':len(paths),'job':'fixture-job-17'})
  if r['method'] in ('query','menu') and (state.parent/'method-error').exists():
   result={'version':1,'id':r['id'],'error':'not-available','message':'State unavailable'}
  with lock:
   if 'error' not in result and not (state.parent/'legacy').exists(): result['revision']=generation
   emit(result)
'''

@pytest.fixture(autouse=True)
def provider_registration(test_environment, fixture_tree, request):
    folder = test_environment.config_home / "strata/providers/example"
    folder.mkdir(parents=True, mode=0o700)
    state = folder / "state"
    state.write_text("on-demand")
    if getattr(request.node, "callspec", None) and request.node.callspec.params.get("legacy"):
        (folder / "legacy").touch()
    script = folder / "provider.py"
    script.write_text(PROGRAM)
    # Small PNG, bounded and loaded only from this trusted registration.
    (folder / "badge.png").write_bytes(base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAHUlEQVR4nGM0qnj2n4ECwESJ5lEDRg0YNWAwGQAAosYCrxCcv1sAAAAASUVORK5CYII="))
    (folder / "provider.json").write_text(json.dumps({"version":1,"id":"example","name":"Example Cloud","command":[sys.executable,str(script),str(fixture_tree.root),str(state),str(folder/"record")],"icons":{"badge":"badge.png"}}))
    fixture_tree.path("local-only.txt").write_text("unrelated")
    return folder


def test_provider_updates_open_menu_and_revalidates_mixed_selection(strata, provider_registration):
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Keep test pin" in strata.menu_items(), "asynchronous provider menu")
    strata.wait(lambda: strata.window.find(role="label", name="Example Cloud"), "provider group heading")
    assert "Example Cloud" not in strata.menu_items(), "provider heading is not an action"
    (provider_registration / "state").write_text("kept")
    strata.wait(lambda: strata.window.find(role="image", description="example: Available offline"), "badge accessibility after state event")
    strata.wait(lambda: "Release test pin" in strata.menu_items(), "event refresh of open menu")
    assert "Keep test pin" not in strata.menu_items()
    (provider_registration / "state").write_text("offline")
    strata.wait(lambda: "Release test pin" not in strata.menu_items(), "provider withdrawal")
    strata.wait(lambda: not strata.window.find(role="label", name="Example Cloud"), "empty provider group withdrawn")
    assert {"Open", "Cut", "Copy"}.issubset(strata.menu_items()), "provider withdrawal preserves built-in commands"
    strata.wait(lambda: not strata.window.find(role="image", description="example: Available offline"), "badges withdrawn")
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
    strata.wait(lambda: strata.window.find(role="image", description="example: Available offline"), "initial badge")
    if invalidate:
        (provider_registration / "event").write_text("refresh without changing state")
    deadline = time.monotonic() + 7
    while time.monotonic() < deadline:
        assert "Release test pin" in strata.menu_items(), "refresh withdrew an unchanged menu"
        assert strata.window.find(role="image", description="example: Available offline"), "refresh withdrew an unchanged badge"
        time.sleep(.04)


@pytest.mark.parametrize("legacy", [False, True])
def test_continuous_events_do_not_starve_slow_menu_or_badge_replies(strata, provider_registration, legacy):
    if legacy:
        (provider_registration / "legacy").touch()
    (provider_registration / "state").write_text("kept")
    (provider_registration / "delay").touch()
    (provider_registration / "storm").touch()
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Release test pin" in strata.menu_items(), "menu progress during continuous events")
    strata.wait(lambda: strata.window.find(role="image", description="example: Available offline"), "badge progress during continuous events")
    deadline = time.monotonic() + 16
    while time.monotonic() < deadline:
        assert "Release test pin" in strata.menu_items()
        assert strata.window.find(role="image", description="example: Available offline")
        time.sleep(.1)


def test_nested_menu_refresh_preserves_navigation_and_activates_only_leaves(strata, provider_registration):
    (provider_registration / "tree").touch()
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Offline availability" in strata.menu_items(), "nested provider menu")
    strata.wait(lambda: strata.window.find(role="label", name="Example Cloud"), "mixed provider group heading")
    assert "Inspect test state" in strata.menu_items()
    strata.pointer.click(strata.menu_item("Offline availability"))
    strata.wait(lambda: "Keep test pin" in strata.menu_items(), "submenu opened")
    assert not (provider_registration / "record").exists(), "submenu navigation dispatched an action"
    (provider_registration / "state").write_text("kept")
    strata.wait(lambda: "Release test pin" in strata.menu_items(), "open submenu refreshed")
    assert "Keep test pin" not in strata.menu_items()
    (provider_registration / "relabel").touch()
    (provider_registration / "hide-inspect").touch()
    (provider_registration / "event").write_text("retained branch renamed and sibling withdrawn")
    strata.wait(lambda: "Release updated pin" in strata.menu_items(), "navigation retained across branch and sibling changes")
    strata.pointer.click(strata.menu_item("Details"))
    strata.wait(lambda: "Inspect nested state" in strata.menu_items(), "recursive submenu")
    (provider_registration / "event").write_text("same tree")
    time.sleep(.5)
    assert "Inspect nested state" in strata.menu_items()
    (provider_registration / "deep-relabel").touch()
    (provider_registration / "event").write_text("two open branches renamed")
    strata.wait(lambda: "Inspect refreshed state" in strata.menu_items(), "recursive navigation retained across ancestor changes")
    (provider_registration / "state").write_text("offline")
    strata.wait(lambda: strata.window.find(role="menu item", name="Inspect refreshed state") is None, "nested actions withdrawn")
    assert not (provider_registration / "record").exists()
    strata.dismiss_menu()
    (provider_registration / "relabel").unlink()
    (provider_registration / "hide-inspect").unlink()
    (provider_registration / "deep-relabel").unlink()
    (provider_registration / "state").write_text("on-demand")
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Offline availability" in strata.menu_items(), "menu restored")
    strata.pointer.click(strata.menu_item("Offline availability"))
    strata.wait(lambda: "Keep test pin" in strata.menu_items(), "leaf available")
    strata.choose_menu_item("Keep test pin")
    record = provider_registration / "record"
    strata.wait(record.exists, "leaf activated")
    assert json.loads(record.read_text())["action"] == "keep"


@pytest.fixture
def overlapping_provider(provider_registration, fixture_tree):
    folder = provider_registration.parent / "beta"
    folder.mkdir(mode=0o700)
    (folder / "state").write_text("kept")
    (folder / "priority").write_text("2")
    (folder / "provider.py").write_text(PROGRAM)
    (folder / "badge.png").write_bytes((provider_registration / "badge.png").read_bytes())
    (folder / "provider.json").write_text(json.dumps({"version": 1, "id": "beta", "command": [sys.executable, str(folder / "provider.py"), str(fixture_tree.root), str(folder / "state"), str(folder / "record")], "icons": {"badge": "badge.png"}}))
    (provider_registration / "state").write_text("kept")
    return folder


@pytest.mark.usefixtures("overlapping_provider")
def test_overlapping_providers_expose_source_and_prioritize_warning_status(strata, provider_registration, overlapping_provider):
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: strata.menu_items().count("Release test pin") == 2, "both providers retain their identically labelled actions")
    strata.wait(lambda: strata.window.find(role="label", name="Example Cloud") and strata.window.find(role="label", name="beta"), "separate provider groups with display name and legacy id fallback")
    strata.wait(lambda: strata.window.find(role="image", description="beta: Available offline; example: Available offline"), "warning provider first with both status descriptions")
    (provider_registration / "state").write_text("offline")
    strata.wait(lambda: not strata.window.find(role="label", name="Example Cloud"), "only the withdrawn provider group disappears")
    strata.wait(lambda: strata.menu_items().count("Release test pin") == 1, "other provider action remains available")
    assert strata.window.find(role="label", name="beta")
    assert {"Open", "Cut", "Copy"}.issubset(strata.menu_items()), "group removal preserves surrounding commands"
    strata.choose_menu_item("Release test pin")
    record = overlapping_provider / "record"
    strata.wait(record.exists, "remaining provider action dispatched")
    assert json.loads(record.read_text())["action"] == "release"
    assert not (provider_registration / "record").exists()


def test_open_menu_replaces_context_without_replacing_leaf(strata, provider_registration):
    token = provider_registration / "context"
    token.write_text("old-object-incarnation")
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Keep test pin" in strata.menu_items(), "token-bearing action")
    token.write_text("new-object-incarnation")
    (provider_registration / "event").write_text("selection identity changed")
    # An unchanged label does not expose the refresh; wait for a full fallback period.
    time.sleep(6)
    strata.choose_menu_item("Keep test pin")
    record = provider_registration / "record"
    strata.wait(record.exists, "context-bearing activation")
    assert json.loads(record.read_text())["context"] == "new-object-incarnation"


def test_partial_activation_reports_original_whole_selection(strata, provider_registration):
    (provider_registration / "partial").touch()
    strata.select_entry("todo.txt")
    strata.click_entry_with("readme.md", ["ctrl"])
    strata.open_context_menu("readme.md")
    strata.wait(lambda: "Keep test pin" in strata.menu_items(), "whole selection action")
    strata.choose_menu_item("Keep test pin")
    strata.wait(lambda: any(" ".join(node.name.split()) == "example: Partially accepted (1/2) Job: fixture-job-17 One test pin accepted; another was rejected" for node in strata.window.find_all(role="label")), "partial counts, job reference and provider source")
    record = json.loads((provider_registration / "record").read_text())
    assert set(record["paths"]) == {str(strata.fixture.path(name)) for name in ("todo.txt", "readme.md")}


def test_method_error_without_revision_withdraws_cached_presentation(strata, provider_registration):
    (provider_registration / "state").write_text("kept")
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Release test pin" in strata.menu_items(), "initial revisioned menu")
    strata.wait(lambda: strata.window.find(role="image", description="example: Available offline"), "initial badge")
    pid = (provider_registration / "pid").read_text()
    (provider_registration / "method-error").touch()
    (provider_registration / "event").write_text("withdraw")
    strata.wait(lambda: "Release test pin" not in strata.menu_items(), "method error withdraws menu")
    strata.wait(lambda: not strata.window.find(role="image", description="example: Available offline"), "method error withdraws badges")
    (provider_registration / "method-error").unlink()
    (provider_registration / "event").write_text("recover")
    strata.wait(lambda: "Release test pin" in strata.menu_items(), "healthy provider recovers without restart")
    strata.wait(lambda: strata.window.find(role="image", description="example: Available offline"), "recovered badge")
    assert (provider_registration / "pid").read_text() == pid


def test_disconnect_after_acceptance_reports_uncertainty_and_never_replays(strata, provider_registration):
    (provider_registration / "disconnect").touch()
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Keep test pin" in strata.menu_items(), "initial action")
    strata.choose_menu_item("Keep test pin")
    record = provider_registration / "record"
    strata.wait(record.exists, "fixture accepted action before disconnect")
    accepted = record.read_text()
    strata.wait(lambda: any(" ".join(node.name.split()) == "example: The provider disconnected. The action may already have been accepted; check its state before retrying." for node in strata.window.find_all(role="label")), "uncertain outcome")
    strata.keyboard.press("Escape")
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: "Release test pin" in strata.menu_items(), "provider restarted and state revalidated")
    assert record.read_text() == accepted, "accepted action was replayed after reconnect"
