#!/usr/bin/env python3
"""UDP bridge that translates codexOS Discord requests to Discord REST API calls.

Protocol from guest (single datagram, UTF-8):
  SYNC\t<bot_token>\t<guild_id>\t<channel_id>\t<after_message_id>\n
  SEND\t<bot_token>\t<channel_id>\t<content>\n
Response to guest (single datagram, UTF-8):
  OK\n
  G\t<guild_id>\t<guild_name>\n
  C\t<channel_id>\t<channel_name>\n
  M\t<message_id>\t<author>\t<content>\n
  E\n
Or on error:
  ERR\t<message>\n
Notes:
- Message content is sanitized (tabs/newlines removed) and truncated.
- The bridge keeps responses under a safe UDP payload budget.
"""

from __future__ import annotations

import argparse
import json
import logging
import socketserver
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from typing import Any

API_BASE = "https://discord.com/api/v10"
USER_AGENT = "codexos-discord-bridge/1.0"
MAX_RESPONSE_BYTES = 1150
MAX_MESSAGE_LINES = 6
MAX_NAME_LEN = 40
MAX_CONTENT_LEN = 160
MAX_ID_LEN = 40


class BridgeError(Exception):
    pass


@dataclass
class DiscordBridge:
    timeout: float

    def request_json(
        self,
        method: str,
        path: str,
        token: str,
        query: dict[str, str] | None = None,
        body: dict[str, Any] | None = None,
    ) -> Any:
        url = API_BASE + path
        if query:
            url = url + "?" + urllib.parse.urlencode(query)

        payload = None
        headers = {
            "Authorization": f"Bot {token}",
            "User-Agent": USER_AGENT,
        }
        if body is not None:
            payload = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"

        req = urllib.request.Request(url, data=payload, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                raw = resp.read()
        except urllib.error.HTTPError as exc:
            body_text = ""
            try:
                body_text = exc.read().decode("utf-8", errors="replace")
            except Exception:
                body_text = ""

            detail = f"discord http {exc.code}"
            if body_text:
                try:
                    payload_obj = json.loads(body_text)
                    message = payload_obj.get("message")
                    if isinstance(message, str) and message:
                        detail = sanitize_field(message, 80)
                except json.JSONDecodeError:
                    pass
            raise BridgeError(detail) from exc
        except urllib.error.URLError as exc:
            raise BridgeError("discord unreachable") from exc

        if not raw:
            return None
        try:
            return json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise BridgeError("invalid discord response") from exc

    def sync(
        self,
        token: str,
        guild_hint: str,
        channel_hint: str,
        after_message_id: str,
    ) -> bytes:
        guilds = self.request_json("GET", "/users/@me/guilds", token, query={"limit": "50"})
        if not isinstance(guilds, list) or not guilds:
            raise BridgeError("no guild access")

        selected_guild = None
        for guild in guilds:
            if not isinstance(guild, dict):
                continue
            gid = str(guild.get("id", ""))
            if guild_hint and gid == guild_hint:
                selected_guild = guild
                break

        if selected_guild is None:
            selected_guild = next((g for g in guilds if isinstance(g, dict)), None)

        if selected_guild is None:
            raise BridgeError("no guild selected")

        guild_id = sanitize_id(str(selected_guild.get("id", "")))
        guild_name = sanitize_field(str(selected_guild.get("name", "guild")), MAX_NAME_LEN)
        if not guild_id:
            raise BridgeError("invalid guild id")

        channels = self.request_json("GET", f"/guilds/{guild_id}/channels", token)
        if not isinstance(channels, list) or not channels:
            raise BridgeError("no channels")

        selected_channel = None
        for channel in channels:
            if not isinstance(channel, dict):
                continue
            cid = str(channel.get("id", ""))
            ctype = channel.get("type")
            if channel_hint and cid == channel_hint and ctype == 0:
                selected_channel = channel
                break

        if selected_channel is None:
            selected_channel = next(
                (
                    c
                    for c in channels
                    if isinstance(c, dict) and c.get("type") == 0
                ),
                None,
            )

        if selected_channel is None:
            raise BridgeError("no text channel")

        channel_id = sanitize_id(str(selected_channel.get("id", "")))
        channel_name = sanitize_field(str(selected_channel.get("name", "channel")), MAX_NAME_LEN)
        if not channel_id:
            raise BridgeError("invalid channel id")

        message_query = {"limit": str(MAX_MESSAGE_LINES)}
        if after_message_id:
            message_query["after"] = after_message_id

        messages = self.request_json(
            "GET",
            f"/channels/{channel_id}/messages",
            token,
            query=message_query,
        )
        if not isinstance(messages, list):
            raise BridgeError("invalid message list")

        # Discord returns newest-first. Return oldest-first so guest appends naturally.
        messages.reverse()

        lines: list[str] = ["OK", f"G\t{guild_id}\t{guild_name}", f"C\t{channel_id}\t{channel_name}"]

        for message in messages[:MAX_MESSAGE_LINES]:
            if not isinstance(message, dict):
                continue

            mid = sanitize_id(str(message.get("id", "")))
            if not mid:
                continue

            author_obj = message.get("author")
            if isinstance(author_obj, dict):
                author = sanitize_field(str(author_obj.get("username", "unknown")), MAX_NAME_LEN)
            else:
                author = "unknown"

            content = sanitize_field(str(message.get("content", "")), MAX_CONTENT_LEN)

            candidate = f"M\t{mid}\t{author}\t{content}"
            encoded = ("\n".join(lines + [candidate, "E"]) + "\n").encode("utf-8")
            if len(encoded) > MAX_RESPONSE_BYTES:
                break
            lines.append(candidate)

        lines.append("E")
        payload = "\n".join(lines) + "\n"
        return payload.encode("utf-8")

    def send(self, token: str, channel_id: str, content: str) -> bytes:
        channel_id = sanitize_id(channel_id)
        if not channel_id:
            raise BridgeError("missing channel id")

        content = sanitize_field(content, MAX_CONTENT_LEN)
        if not content:
            raise BridgeError("empty message")

        payload = self.request_json(
            "POST",
            f"/channels/{channel_id}/messages",
            token,
            body={"content": content},
        )
        if not isinstance(payload, dict):
            raise BridgeError("invalid send response")

        message_id = sanitize_id(str(payload.get("id", "")))
        if not message_id:
            raise BridgeError("missing message id")

        return f"SENT\t{message_id}\n".encode("utf-8")


class BridgeHandler(socketserver.BaseRequestHandler):
    bridge: DiscordBridge

    def handle(self) -> None:
        packet, sock = self.request
        response = b""
        try:
            response = self.process(packet)
        except BridgeError as exc:
            response = f"ERR\t{sanitize_field(str(exc), 80)}\n".encode("utf-8")
        except Exception as exc:  # defensive catch to keep server alive
            logging.exception("unexpected bridge error")
            response = f"ERR\t{sanitize_field(str(exc), 80)}\n".encode("utf-8")

        if len(response) > MAX_RESPONSE_BYTES:
            response = b"ERR\tresponse too large\n"

        sock.sendto(response, self.client_address)

    def process(self, packet: bytes) -> bytes:
        if not packet:
            raise BridgeError("empty request")

        text = packet.decode("utf-8", errors="replace").strip("\r\n")
        parts = text.split("\t")
        if not parts:
            raise BridgeError("invalid request")

        command = parts[0].strip().upper()

        if command == "SYNC":
            if len(parts) < 5:
                raise BridgeError("sync requires token/guild/channel/after")
            token = sanitize_token(parts[1])
            guild_id = sanitize_id(parts[2])
            channel_id = sanitize_id(parts[3])
            after_id = sanitize_id(parts[4])
            if not token:
                raise BridgeError("missing bot token")
            return self.bridge.sync(token, guild_id, channel_id, after_id)

        if command == "SEND":
            if len(parts) < 4:
                raise BridgeError("send requires token/channel/content")
            token = sanitize_token(parts[1])
            channel_id = sanitize_id(parts[2])
            content = sanitize_field(parts[3], MAX_CONTENT_LEN)
            if not token:
                raise BridgeError("missing bot token")
            return self.bridge.send(token, channel_id, content)

        raise BridgeError("unknown command")


def sanitize_token(value: str) -> str:
    return "".join(ch for ch in value if 0x21 <= ord(ch) <= 0x7E)


def sanitize_id(value: str) -> str:
    cleaned = "".join(ch for ch in value if ch.isdigit())
    return cleaned[:MAX_ID_LEN]


def sanitize_field(value: str, limit: int) -> str:
    out = []
    for ch in value:
        if ch in "\r\n\t":
            out.append(" ")
        elif 0x20 <= ord(ch) <= 0x7E:
            out.append(ch)
        else:
            out.append("?")
        if len(out) >= limit:
            break
    return "".join(out).strip()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="codexOS Discord UDP bridge")
    parser.add_argument("--bind", default="0.0.0.0", help="UDP bind address (default: 0.0.0.0)")
    parser.add_argument("--port", type=int, default=4242, help="UDP port (default: 4242)")
    parser.add_argument(
        "--timeout",
        type=float,
        default=8.0,
        help="Discord API timeout in seconds (default: 8)",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Enable request logging",
    )
    return parser


def main(argv: list[str]) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
    )

    if args.port <= 0 or args.port > 65535:
        print("invalid port", file=sys.stderr)
        return 2

    bridge = DiscordBridge(timeout=max(1.0, args.timeout))

    class _Handler(BridgeHandler):
        pass

    _Handler.bridge = bridge

    with socketserver.ThreadingUDPServer((args.bind, args.port), _Handler) as server:
        logging.info("discord bridge listening on %s:%d", args.bind, args.port)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            logging.info("stopped")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
