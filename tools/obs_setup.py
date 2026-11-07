#!/usr/bin/env python3
"""实际接入 OBS：创建/更新 EEW 叠加浏览器源，可选抓取场景截图验证。

用法：
    python tools/obs_setup.py                       # 创建/更新来源
    python tools/obs_setup.py --screenshot out.png  # 并抓一张 OBS 场景截图
    python tools/obs_setup.py --hide                # 隐藏来源

直接读取 OBS 自己的 obs-websocket 配置（密码/端口），不需要额外配置。
只依赖标准库：WebSocket 帧、SHA256 认证全部手写，与 Rust 侧 obsws.rs 对齐。
"""
import argparse
import base64
import hashlib
import json
import os
import socket
import struct
import sys
import time

APPDATA = os.environ.get("APPDATA", "")
OBS_CFG = os.path.join(APPDATA, "obs-studio", "plugin_config", "obs-websocket", "config.json")
SOURCE_NAME = "EEW-Overlay"
DEFAULT_URL = "http://127.0.0.1:18787/obs"


def obs_config():
    with open(OBS_CFG, encoding="utf-8") as f:
        c = json.load(f)
    if not c.get("server_enabled"):
        sys.exit("obs-websocket 未启用：OBS → 工具 → WebSocket 服务器设置 → 启用")
    return c.get("server_password", ""), int(c.get("server_port", 4455))


class WS:
    def __init__(self, host, port):
        self.s = socket.create_connection((host, port), timeout=5)
        self.s.settimeout(5)
        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET / HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self.s.sendall(req.encode())
        self.buf = b""
        while b"\r\n\r\n" not in self.buf:
            self.buf += self.s.recv(4096)
        head, self.buf = self.buf.split(b"\r\n\r\n", 1)
        if not head.lower().startswith(b"http/1.1 101"):
            sys.exit("WebSocket 升级失败: " + head.decode(errors="replace"))

    def _read(self, n):
        while len(self.buf) < n:
            chunk = self.s.recv(4096)
            if not chunk:
                raise ConnectionError("连接已关闭")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send(self, obj):
        data = json.dumps(obj, ensure_ascii=False).encode()
        head = bytearray([0x81])
        n = len(data)
        if n < 126:
            head.append(0x80 | n)
        elif n < 65536:
            head.append(0x80 | 126)
            head += struct.pack(">H", n)
        else:
            head.append(0x80 | 127)
            head += struct.pack(">Q", n)
        mask = os.urandom(4)
        head += mask
        body = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        self.s.sendall(bytes(head) + body)

    def recv(self):
        while True:
            b0, b1 = self._read(2)
            op = b0 & 0x0F
            n = b1 & 0x7F
            if n == 126:
                n = struct.unpack(">H", self._read(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self._read(8))[0]
            if b1 & 0x80:
                mask = self._read(4)
                data = bytes(b ^ mask[i % 4] for i, b in enumerate(self._read(n)))
            else:
                data = self._read(n)
            if op == 1:
                return json.loads(data.decode("utf-8"))
            if op == 8:
                raise ConnectionError("服务端关闭连接")

    def hello(self):
        msg = self.recv()
        if msg.get("op") != 0:
            sys.exit("首帧不是 Hello: " + str(msg)[:200])
        return msg["d"]

    def identify(self, password):
        hello = self.hello()
        d = {"rpcVersion": 1, "eventSubscriptions": 0}
        auth = hello.get("authentication")
        if auth:
            if not password:
                sys.exit("OBS 要求密码，但配置里读不到")
            secret = base64.b64encode(
                hashlib.sha256((password + auth["salt"]).encode()).digest()).decode()
            resp = base64.b64encode(
                hashlib.sha256((secret + auth["challenge"]).encode()).digest()).decode()
            d["authentication"] = resp
        self.send({"op": 1, "d": d})
        while True:
            m = self.recv()
            if m.get("op") == 2:
                return
            if m.get("op") == 5:
                sys.exit("Identify 被拒绝：" + str(m)[:200])

    def request(self, kind, data=None, rid=None):
        rid = rid or ("py-" + kind)
        self.send({"op": 6, "d": {"requestType": kind, "requestId": rid,
                                  "requestData": data or {}}})
        while True:
            m = self.recv()
            if m.get("op") == 7 and m["d"].get("requestId") == rid:
                st = m["d"].get("requestStatus", {})
                if not st.get("result"):
                    raise RuntimeError(f"{kind}: {st.get('comment') or st}")
                return m["d"].get("responseData", {})


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default=DEFAULT_URL)
    ap.add_argument("--screenshot", help="保存当前场景截图到该 PNG 路径")
    ap.add_argument("--hide", action="store_true", help="隐藏叠加来源")
    ap.add_argument("--show", action="store_true", help="显示叠加来源")
    ap.add_argument("--shot-only", action="store_true", help="只抓截图，不动来源（避免刷新页面）")
    ap.add_argument("--name", default=SOURCE_NAME)
    ap.add_argument("--port", type=int, default=18787)
    args = ap.parse_args()

    password, port = obs_config()
    ws = WS("127.0.0.1", port)
    ws.identify(password)
    print("connected to obs-websocket v5")

    scene = ws.request("GetCurrentProgramScene")["currentProgramSceneName"]
    print("scene:", scene)

    # 浏览器源设置：透明背景、1920x1080、把 Web Audio 提示音送进 OBS 混音器
    settings = {
        "url": args.url,
        "width": 1920,
        "height": 1080,
        "css": "html,body{background:transparent !important}",
        "reroute_audio": True,
        "restart_when_active": False,
        "shutdown_when_not_active": False,
        "fps": 30,
    }
    if not args.shot_only:
        try:
            item = ws.request("GetSceneItemId", {"sceneName": scene, "sourceName": args.name})
            item_id = item["sceneItemId"]
            ws.request("SetInputSettings", {"inputName": args.name, "inputSettings": settings})
            print(f"updated existing source '{args.name}' (id={item_id})")
        except RuntimeError:
            item = ws.request("CreateInput", {
                "sceneName": scene,
                "inputName": args.name,
                "inputKind": "browser_source",
                "inputSettings": settings,
                "sceneItemEnabled": True,
            })
            item_id = item["sceneItemId"]
            print(f"created source '{args.name}' (id={item_id})")

        if args.hide or args.show:
            ws.request("SetSceneItemEnabled", {"sceneName": scene, "sceneItemId": item_id,
                                              "sceneItemEnabled": bool(args.show)})
            print("source", "shown" if args.show else "hidden")

    if args.screenshot:
        time.sleep(1.5)
        r = ws.request("GetSourceScreenshot", {
            "sourceName": scene, "imageFormat": "png",
            "imageWidth": 1920, "imageHeight": 1080,
        })
        img = r["imageData"].split(",", 1)[-1]
        with open(args.screenshot, "wb") as f:
            f.write(base64.b64decode(img))
        print("screenshot saved:", args.screenshot)

    print("OBS URL:", args.url)
    print("done")


if __name__ == "__main__":
    main()
