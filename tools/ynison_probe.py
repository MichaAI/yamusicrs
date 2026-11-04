"""Ynison probe: register as a passive player device (like the Music Assistant provider) and dump hub messages.

Usage: python3 ynison_probe.py TOKEN_JSON SECONDS [OUT_JSON]

Registers with an empty queue whose versions have timestamp 0 and `is_currently_active: false`,
so it never takes over playback; it only appears in the device list and listens.
"""
import asyncio, json, sys, time, uuid

import aiohttp

TOKEN = json.load(open(sys.argv[1]))["access_token"]
SECONDS = float(sys.argv[2]) if len(sys.argv) > 2 else 15
OUT = sys.argv[3] if len(sys.argv) > 3 else None
DEVICE_ID = sys.argv[4] if len(sys.argv) > 4 else "yamusicrs-probe-" + uuid.uuid4().hex[:8]
TITLE = "yamusicrs"
REDIRECT = "wss://ynison.music.yandex.ru/redirector.YnisonRedirectService/GetRedirectToYnison"
STATE_PATH = "/ynison_state.YnisonStateService/PutYnisonState"


def headers(ticket=None, session=None):
    proto = {
        "Ynison-Device-Id": DEVICE_ID,
        "Ynison-Device-Info": json.dumps({"app_name": TITLE, "app_version": "0.1.0", "type": 1}),
    }
    if ticket:
        proto["Ynison-Redirect-Ticket"] = ticket
        proto["Ynison-Session-Id"] = str(session)
    return {
        "Authorization": f"OAuth {TOKEN}",
        "Origin": "https://music.yandex.ru",
        "Sec-WebSocket-Protocol": f"Bearer, v2, {json.dumps(proto)}",
    }


def version():
    return {"device_id": DEVICE_ID, "version": str(time.time_ns()), "timestamp_ms": "0"}


def meta():
    return {
        "rid": str(uuid.uuid4()),
        "player_action_timestamp_ms": str(int(time.time() * 1000)),
        "activity_interception_type": "DO_NOT_INTERCEPT_BY_DEFAULT",
    }


def summary(m):
    ps = m.get("player_state") or {}
    q = ps.get("player_queue") or {}
    st = ps.get("status") or {}
    pl = q.get("playable_list") or []
    cur = q.get("current_playable_index")
    try:
        title = pl[int(cur)].get("title") if pl and cur is not None and 0 <= int(cur) < len(pl) else None
    except (TypeError, ValueError):
        title = None
    return (f"active={m.get('active_device_id_optional')!r} entity={q.get('entity_type')}:{q.get('entity_id')} "
            f"idx={cur}/{len(pl)} now={title!r} paused={st.get('paused')} "
            f"progress={st.get('progress_ms')}/{st.get('duration_ms')} "
            f"status_by={(st.get('version') or {}).get('device_id')!r} queue_by={(q.get('version') or {}).get('device_id')!r}")


async def main():
    async with aiohttp.ClientSession() as s:
        async with s.ws_connect(REDIRECT, headers=headers()) as ws:
            redir = json.loads((await ws.receive(timeout=10)).data)
        if "error" in redir or not redir.get("host"):
            print("REDIRECT ERROR", redir)
            return
        print("redirected to", redir["host"], "keepalive", redir.get("keep_alive_params"))
        url = f"wss://{redir['host']}{STATE_PATH}"
        async with s.ws_connect(url, headers=headers(redir["redirect_ticket"], redir["session_id"]), heartbeat=30) as ws:
            await ws.send_str(json.dumps({
                "update_full_state": {
                    "player_state": {
                        "status": {"paused": True, "duration_ms": "0", "progress_ms": "0", "playback_speed": 1, "version": version()},
                        "player_queue": {"current_playable_index": -1, "entity_id": "", "entity_type": "VARIOUS",
                                         "playable_list": [], "options": {"repeat_mode": "NONE"},
                                         "entity_context": "BASED_ON_ENTITY_BY_DEFAULT", "version": version(), "from_optional": ""},
                    },
                    "device": {
                        "info": {"device_id": DEVICE_ID, "title": TITLE, "type": "WEB", "app_name": TITLE, "app_version": "0.1.0"},
                        "capabilities": {"can_be_player": True, "can_be_remote_controller": False},
                    },
                    "is_currently_active": False,
                },
                **meta(),
            }))
            msgs = []
            end = time.time() + SECONDS
            while time.time() < end:
                try:
                    msg = await ws.receive(timeout=max(0.1, end - time.time()))
                except asyncio.TimeoutError:
                    break
                if msg.type != aiohttp.WSMsgType.TEXT:
                    print("ws", msg.type, msg.data)
                    break
                m = json.loads(msg.data)
                msgs.append({"t": time.time(), "msg": m})
                if "error" in m:
                    print("HUB ERROR", json.dumps(m, ensure_ascii=False))
                    break
                print(time.strftime("%T"), summary(m))
                for d in m.get("devices") or []:
                    i = d.get("info") or {}
                    print(f"   device {i.get('title')!r} {i.get('type')} {i.get('app_name')} id={i.get('device_id')} "
                          f"{'offline' if d.get('is_offline') else 'online'} caps={d.get('capabilities')}")
            if OUT:
                json.dump(msgs, open(OUT, "w"), ensure_ascii=False, indent=1)


asyncio.run(main())
