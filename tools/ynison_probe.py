"""Ynison probe: connect like desktop 5.63.1 in shadow mode and dump what the hub sends."""
import asyncio, json, sys, time, uuid, random, urllib.parse
import websockets

TOKEN = json.load(open(sys.argv[1]))["access_token"]
SECONDS = float(sys.argv[2]) if len(sys.argv) > 2 else 15
OUT = sys.argv[3] if len(sys.argv) > 3 else None
DEVICE_ID = "yamusicrs-probe-" + uuid.uuid4().hex[:8]
INFO = {"app_name": "Browser", "app_version": "5.95.0", "type": 1}


def proto(extra):
    h = {"Ynison-Device-Id": DEVICE_ID, "Ynison-Device-Info": json.dumps(INFO), "authorization": f"OAuth {TOKEN}"}
    h.update(extra)
    return ["Bearer", "v2", urllib.parse.quote(json.dumps(h), safe="")]


def version(ts=None):
    return {"device_id": DEVICE_ID, "version": random.randrange(2**63), "timestamp_ms": int(time.time() * 1000) if ts is None else ts}


async def main():
    async with websockets.connect(
        "wss://ynison.music.yandex.ru/redirector.YnisonRedirectService/GetRedirectToYnison",
        subprotocols=proto({}), origin="https://music.yandex.ru",
    ) as ws:
        redir = json.loads(await ws.recv())
    print("redirector keys:", list(redir.keys()))
    hub = f"wss://{redir['host']}/ynison_state.YnisonStateService/PutYnisonState"
    extra = {"Ynison-Redirect-Ticket": redir["redirect_ticket"], "Ynison-Session-Id": redir["session_id"]}
    async with websockets.connect(hub, subprotocols=proto(extra), origin="https://music.yandex.ru", max_size=None) as ws:
        v0 = version(0)
        rid = str(uuid.uuid4())
        full = {
            "update_full_state": {
                "player_state": {
                    "player_queue": {"current_playable_index": -1, "entity_id": "", "entity_type": "VARIOUS",
                                     "playable_list": [], "options": {"repeat_mode": "NONE"}, "shuffle_optional": None,
                                     "entity_context": "BASED_ON_ENTITY_BY_DEFAULT", "version": v0, "from_optional": "",
                                     "initial_entity_optional": None, "adding_options_optional": None, "queue": None},
                    "status": {"duration_ms": 0, "paused": True, "playback_speed": 1, "progress_ms": 0, "version": v0},
                    "player_queue_inject_optional": None,
                },
                "device": {"volume": 1, "capabilities": {"can_be_player": True, "can_be_remote_controller": False, "volume_granularity": 16},
                           "info": {"app_name": "Browser", "app_version": "5.95.0", "title": "yamusicrs probe", "device_id": DEVICE_ID, "type": "WEB"},
                           "volume_info": {"volume": 0, "version": None}, "is_shadow": True},
                "is_currently_active": False,
                "sync_state_from_eov_optional": None,
            },
            "rid": rid,
            "player_action_timestamp_ms": 0,
            "activity_interception_type": "DO_NOT_INTERCEPT_BY_DEFAULT",
        }
        await ws.send(json.dumps(full))
        msgs = []
        end = time.time() + SECONDS
        while time.time() < end:
            try:
                m = json.loads(await asyncio.wait_for(ws.recv(), timeout=end - time.time()))
            except asyncio.TimeoutError:
                break
            msgs.append(m)
            if 'error' in m:
                print('ERROR', json.dumps(m, ensure_ascii=False)); break
            ps = m.get("player_state", {})
            q = ps.get("player_queue", {})
            st = ps.get("status", {})
            cur = q.get("current_playable_index")
            pl = q.get("playable_list") or []
            title = pl[cur]["title"] if isinstance(cur, int) and 0 <= cur < len(pl) else None
            print(f"msg rid={m.get('rid')} keys={list(m.keys())} active={m.get('active_device_id_optional')} "
                  f"entity={q.get('entity_type')}:{q.get('entity_id')} idx={cur}/{len(pl)} now={title!r} "
                  f"paused={st.get('paused')} progress={st.get('progress_ms')}/{st.get('duration_ms')}")
            for d in m.get("devices", []):
                print("   device", d.get("info", {}).get("title"), d.get("info", {}).get("type"), d["info"].get("device_id"),
                      "offline" if d.get("is_offline") else "online", "shadow" if d.get("is_shadow") else "", d.get("capabilities"))
        if OUT:
            json.dump(msgs, open(OUT, "w"), ensure_ascii=False, indent=1)


asyncio.run(main())
