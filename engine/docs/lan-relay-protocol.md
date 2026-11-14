# LAN play relay protocol, version 1

This document specifies the UDP protocol between the game runtime (the *client*, `runtime/lan_net.c`) and a *relay*
that lets copies of the game on different networks play Splatoon 3's LAN mode together. It is complete on its own: a
relay built from it alone interoperates with the client. The words MUST, MUST NOT, SHOULD and MAY are used as in
RFC 2119. [lan-play.md](lan-play.md) describes the game side.

## 1. Model

- The relay is one UDP endpoint (default port **47313**). It knows nothing of the game: it forwards opaque datagrams.
- A *room* is a virtual LAN: the set of clients that joined with the same room code. Rooms are isolated: nothing is
  ever forwarded from one room to another.
- A *member* is a client that completed the join handshake. It has a *session id* (u32), a secret *token* (8 bytes),
  a *virtual IP address* (vip) inside the virtual subnet **10.13.0.0/16**, and a *transport address* (the IP address
  and UDP port its datagrams come from).
- The game inside a client sees only the virtual subnet: its own vip with netmask 255.255.0.0 and broadcast address
  10.13.255.255. Every datagram the game sends is carried in a DATA message to the relay, which delivers it to the
  member with the destination vip (unicast) or to every other member of the room (broadcast).
- The relay forwards only between members. It never sends a datagram to an address that has not proved it receives
  datagrams there (section 7), and it never sends more bytes to an unproven address than it received from it.

## 2. Transport and encoding

- UDP. One message per datagram. A relay MUST accept IPv4 and MAY also listen on IPv6 (the current client uses IPv4).
- The largest datagram either side sends or accepts is **1472 bytes** (the UDP payload of a 1500-byte IPv4 packet).
  A receiver MUST drop a datagram larger than that.
- All integers are unsigned and **big-endian**. Offsets are in bytes from the start of the datagram.
- Reserved fields MUST be sent as zero and MUST be ignored on receipt.
- Padding is zero bytes at the end of a message; receivers MUST ignore bytes past the fields they need, except where
  a minimum size is stated.

### 2.1 Common header (8 bytes)

| Offset | Size | Field | Value |
|---|---|---|---|
| 0 | 4 | magic | ASCII `S3LR` (53 33 4C 52) |
| 4 | 1 | version | 1 |
| 5 | 1 | type | section 3 |
| 6 | 2 | reserved | 0 |

A datagram shorter than 8 bytes, or with another magic, MUST be dropped silently. A datagram with another version
MUST be dropped, except that a relay answers a HELLO of another version with JOIN_NAK reason 1 (section 6.1).

### 2.2 Session header (20 bytes)

Every message from KEEPALIVE (type 0x06) on starts with the common header followed by:

| Offset | Size | Field |
|---|---|---|
| 8 | 4 | session id |
| 12 | 8 | token |

From client to relay these are the sender's own. From relay to client they are the *recipient's*, so a client checks
that every relay message carries its own session id and token and drops it otherwise.

## 3. Messages

| Type | Name | Direction | Size (bytes) |
|---|---|---|---|
| 0x01 | HELLO | client to relay | exactly 64 (at least 64 accepted) |
| 0x02 | CHALLENGE | relay to client | 32 |
| 0x03 | JOIN | client to relay | 40 + room + name, padded to at least 64; at most 104 |
| 0x04 | JOIN_ACK | relay to client | 40 |
| 0x05 | JOIN_NAK | relay to client | 24 |
| 0x06 | KEEPALIVE | client to relay | 28 |
| 0x07 | KEEPALIVE_ACK | relay to client | 28 |
| 0x08 | LEAVE | client to relay | 20 |
| 0x09 | CLOSED | relay to client | 24 |
| 0x10 | DATA | both | 32 + payload, payload 0..1440 |
| 0x11 | MEMBER | relay to client | 28 + name |

A receiver MUST drop a message shorter than its size above (minimum sizes for HELLO and JOIN), a message of an
unknown type, and a message that comes the wrong way (for example a CHALLENGE at the relay).

Types 0x20 and 0x21 are taken by the reference relay for its status question and answer between a relay and the
community service bound to it (the server repository's `docs/protocol/relay.md`, section 4.3): never sent to or by a
game. A client never sends them; a relay that does not know them drops them as unknown types.

### 3.1 HELLO (0x01), client to relay

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | common header, type 0x01 |
| 8 | 8 | client nonce: random, chosen by the client for this handshake |
| 16 | 48 | zero padding |

The padding makes HELLO larger than every answer to it.

### 3.2 CHALLENGE (0x02), relay to client

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | common header, type 0x02 |
| 8 | 8 | client nonce, copied from the HELLO |
| 16 | 16 | cookie (opaque to the client) |

### 3.3 JOIN (0x03), client to relay

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | common header, type 0x03 |
| 8 | 8 | client nonce (the HELLO's) |
| 16 | 16 | cookie, copied from the CHALLENGE |
| 32 | 4 | requested vip: 0 for any, else the address the client wants (its previous one when it joins again) |
| 36 | 1 | room length R, 1..32 |
| 37 | 1 | name length N, 0..32 |
| 38 | 2 | reserved |
| 40 | R | room code: bytes `A`-`Z`, `0`-`9`, `-`, `_` (upper case only) |
| 40+R | N | display name: any bytes but 0x00-0x1F and 0x7F (meant as UTF-8; informational only) |
| 40+R+N | | zero padding up to 64 bytes when 40+R+N < 64 |

### 3.4 JOIN_ACK (0x04), relay to client

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | common header, type 0x04 |
| 8 | 8 | client nonce, copied from the JOIN |
| 16 | 4 | session id: non-zero, unique among the relay's live sessions |
| 20 | 8 | token: 8 bytes from a cryptographically secure random source |
| 28 | 4 | vip assigned |
| 32 | 1 | prefix length: 16 |
| 33 | 1 | members at most in the room |
| 34 | 1 | keepalive interval in seconds (default 5, 1..60) |
| 35 | 1 | idle timeout in seconds (default 30; at least 3 times the keepalive interval) |
| 36 | 2 | largest DATA payload accepted: 1440 |
| 38 | 2 | reserved |

The client refuses a JOIN_ACK whose vip is outside 10.13.0.0/16, whose last two bytes are 0.0 or 255.255, or whose
prefix length is not 16.

### 3.5 JOIN_NAK (0x05), relay to client

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | common header, type 0x05 (version field: the relay's own version) |
| 8 | 8 | client nonce, copied from the HELLO or JOIN |
| 16 | 1 | reason |
| 17 | 1 | detail (reasons 4 and 5; zero otherwise) |
| 18 | 6 | reserved |

| Reason | Meaning | Client waits before a new HELLO |
|---|---|---|
| 1 | protocol version not supported (answer to a HELLO of another version) | 60 s |
| 2 | cookie invalid or expired | none |
| 3 | invalid room code | 10 s |
| 4 | room full (or no free vip) | 10 s |
| 5 | relay full (too many rooms or sessions) | 10 s |
| 6 | invalid name | 10 s |
| 7 | rate limited | 10 s |
| 8 | malformed JOIN | 10 s |

The detail byte says why a room or the relay is full, for a client that shows it to its player: 0, not said (the
room or the relay is full); 1, the limit counts the JOIN's source address (an IPv4 address or an IPv6 /64): too many
members of this room (reason 4) or sessions of this relay (reason 5) come from it, and the relay's operator can raise
that limit. The byte was reserved (zero) before relays of the server protocol 1.1.13: a client that does not read it
reads reasons 4 and 5 as before, and a client that reads it treats 0 as "not said".

### 3.6 KEEPALIVE (0x06), client to relay

| Offset | Size | Field |
|---|---|---|
| 0 | 20 | session header, type 0x06 |
| 20 | 4 | sequence number (the client counts up from 1) |
| 24 | 4 | reserved (keeps the request as large as the answer) |

### 3.7 KEEPALIVE_ACK (0x07), relay to client

| Offset | Size | Field |
|---|---|---|
| 0 | 20 | session header (the recipient's), type 0x07 |
| 20 | 4 | sequence number, copied from the KEEPALIVE |
| 24 | 1 | members now in the room (255 if more) |
| 25 | 3 | reserved |

### 3.8 LEAVE (0x08), client to relay

The session header alone (20 bytes). The relay ends the session; it sends no answer.

### 3.9 CLOSED (0x09), relay to client

| Offset | Size | Field |
|---|---|---|
| 0 | 20 | session header, type 0x09: the session id and token of the session that ended (for reason 2, copied from the message being answered) |
| 20 | 1 | reason: 1 idle timeout, 2 unknown session, 3 removed by the operator, 4 relay shutting down, 5 protocol violation |
| 21 | 3 | reserved |

On a CLOSED that carries its session id and token, a client starts a new handshake at once.

### 3.10 DATA (0x10), both directions

| Offset | Size | Field |
|---|---|---|
| 0 | 20 | session header, type 0x10 |
| 20 | 4 | source vip |
| 24 | 2 | source UDP port (the game socket's) |
| 26 | 4 | destination vip: a member's vip, or the broadcast address 10.13.255.255 (255.255.255.255 is accepted as the same) |
| 30 | 2 | destination UDP port |
| 32 | 0..1440 | payload: the game's datagram, unchanged |

Client to relay: the session header is the sender's and the source vip MUST be the sender's vip. Relay to client: the
relay copies bytes 20 to the end unchanged and puts the recipient's session id and token in bytes 8-19.

### 3.11 MEMBER (0x11), relay to client (informational)

| Offset | Size | Field |
|---|---|---|
| 0 | 20 | session header (the recipient's), type 0x11 |
| 20 | 1 | event: 1 joined, 2 left |
| 21 | 1 | name length N, 0..32 |
| 22 | 2 | reserved |
| 24 | 4 | the member's vip |
| 28 | N | the member's display name |

When a member joins, the relay sends MEMBER(joined) about it to every other member, and MEMBER(joined) about each
other member to it. When a member leaves or times out, it sends MEMBER(left) to the remaining members. Clients only
log these; a relay MAY omit them.

## 4. Client behavior

States: HELLO, JOIN, JOINED.

1. **HELLO.** Pick a new random nonce; send HELLO. Resend it every 1 s; after 5 unanswered, every 5 s, without end.
2. On a CHALLENGE whose nonce matches, in state HELLO: send JOIN (same nonce, the cookie, the room code, the name,
   the requested vip) and enter **JOIN**. Resend the JOIN every 1 s; after 5 unanswered, go back to HELLO (new nonce).
3. On a JOIN_ACK whose nonce matches, in state JOIN: keep session id, token, vip, keepalive interval; enter **JOINED**.
4. On a JOIN_NAK whose nonce matches, outside JOINED: go back to HELLO after the wait of section 3.5.
5. **JOINED:** send a KEEPALIVE every keepalive interval, whether or not DATA flows. Any relay message carrying the
   client's session id and token counts as a sign of life. With none for 3 keepalive intervals (15 s by default),
   join again (HELLO with a new nonce, requesting the current vip).
6. On CLOSED with the client's session id and token: join again at once, requesting the current vip.
7. When the game ends, send LEAVE (the client sends it twice).
8. The client accepts datagrams from the relay's address and port only. It drops a DATA whose session id or token is
   not its own, whose source vip is outside 10.13.0.0/16 or is its own, or whose destination is neither its vip nor
   the broadcast address.
9. Broadcasts the game sends are also delivered to the game's own sockets by the client (a BSD stack hands the host
   its own broadcasts); the relay does not echo them. Datagrams the game sends to its own vip never leave the client.

## 5. Relay behavior

### 5.1 Handshake

- **HELLO** (at least 64 bytes, version 1): answer CHALLENGE with the HELLO's nonce and the cookie of section 7.1.
  Keep no state.
- **JOIN**: drop it when shorter than 64 bytes. Check, in this order, answering JOIN_NAK with the reason given:
  1. the cookie (section 7.1), for the JOIN's source address and nonce: else reason 2;
  2. lengths: room 1..32, name 0..32, 40+R+N within the datagram: else reason 8;
  3. room code characters: else reason 3; name bytes: else reason 6;
  4. if the room already has a member with this source address and this nonce, the JOIN is a retransmission whose
     JOIN_ACK was lost: send that member's JOIN_ACK again and stop;
  5. if the room has a member with this source address (and another nonce), that client lost its session: end the
     old member silently (MEMBER(left) to the others) before going on;
  6. room capacity (default 12 members; configurable 2..254): else reason 4; relay capacity (rooms, sessions): else
     reason 5; a relay that also limits what one source address may hold (the reference relay: members of one room,
     sessions of the relay) refuses with reason 4 or 5 and detail 1 (section 3.5);
  7. vip: the requested vip if it is inside 10.13.0.0/16, its last byte is neither 0 nor 255, and no member of the
     room holds it; else the lowest free address from 10.13.0.1 upwards whose last byte is neither 0 nor 255; none
     free: reason 4.
  Then create the member (new session id and token), send JOIN_ACK to the JOIN's source address, and send the MEMBER
  messages of section 3.11.

### 5.2 Session messages

For KEEPALIVE, LEAVE and DATA, find the member by session id and compare the token in constant time.

- **Unknown session or wrong token:** drop. For KEEPALIVE and DATA the relay MAY answer CLOSED reason 2 (copying the
  session id and token it received), at most once per second per source address. Never answer a LEAVE.
- **KEEPALIVE** from another source address than the member's: the client moved (NAT rebinding); update the member's
  transport address. Then mark the member alive and answer KEEPALIVE_ACK to the (new) address.
- **LEAVE** and **DATA** MUST come from the member's transport address; drop them otherwise.
- **LEAVE**: end the session (MEMBER(left) to the others).
- **DATA**: drop it when the payload exceeds 1440 bytes, when the source vip is not the member's, or when the
  member's rate limit (section 7.3) is exceeded. Otherwise mark the member alive and forward:
  - destination 10.13.255.255 or 255.255.255.255: one copy to every other member of the room;
  - destination equal to another member's vip: one copy to that member;
  - anything else (unknown vip, the sender's own vip): drop.
  Each copy is the received datagram with bytes 8-19 replaced by the recipient's session id and token.

### 5.3 Timeouts

- A member from whom no valid KEEPALIVE, DATA or LEAVE arrived for the idle timeout (default 30 s) is removed; the
  relay MAY send it CLOSED reason 1, and sends MEMBER(left) to the others.
- A room with no members ceases to exist.
- When shutting down, the relay SHOULD send CLOSED reason 4 to every member.

## 6. Versioning

### 6.1 Other versions

A relay that receives a HELLO (at least 64 bytes) with a version it does not support answers JOIN_NAK reason 1 with
its own version in the header and the HELLO's nonce, and drops every other message of another version. A client
logs the relay's version and retries after 60 s. A future version MUST keep the common header and the HELLO /
JOIN_NAK layouts so that this negotiation keeps working.

## 7. Security and abuse resistance

### 7.1 Cookie: return routability without state

The cookie proves that the client receives datagrams at the source address of its JOIN, so a spoofed source address
cannot make the relay create a session for, or send traffic to, a victim. Recommended construction:

    cookie = first 16 bytes of HMAC-SHA256(secret, address bytes || port (2 bytes) || nonce (8 bytes) || epoch (8 bytes))
    epoch  = floor(unix time in seconds / 30)

`secret` is 32 random bytes chosen when the relay starts. A JOIN's cookie is valid when it matches the current epoch
or the previous one (a cookie lives 30 to 60 seconds). The client treats the cookie as opaque, so a relay MAY use
another construction with the same properties.

### 7.2 No amplification

Before an address has completed the handshake, the relay sends it at most one datagram per datagram received, never
larger:

| Received | Answer |
|---|---|
| HELLO, at least 64 bytes | CHALLENGE 32, or JOIN_NAK 24 |
| JOIN, at least 64 bytes | JOIN_ACK 40 or JOIN_NAK 24 |
| KEEPALIVE 28, DATA at least 32, unknown session | at most CLOSED 24, at most once per second per address |
| anything else | nothing |

After a JOIN_ACK the member's address is proven; the MEMBER messages and forwarded DATA it then receives are traffic
it asked for by joining. Broadcast forwarding multiplies a datagram by at most (room size - 1) toward members only.

### 7.3 Limits (recommended defaults, configurable)

| Limit | Default |
|---|---|
| members per room | 12 (Splatoon 3 LAN: 8 players and 2 spectators, Salmon Run 4) |
| rooms per relay | 256 |
| sessions per relay | 1024 |
| HELLO and JOIN per source IP address | 20 per second each, excess dropped |
| DATA per member | 2000 datagrams and 4 MiB per second (token buckets one second deep), excess dropped |
| DATA payload | 1440 bytes |

### 7.4 What the protocol does not protect

- The protocol adds no encryption: payloads travel between client and relay as the game's session library made
  them. The relay never reads payloads.
- The room code is the only admission control: anyone who knows it can join. Public relays SHOULD tell players to use
  long random room codes. A relay MAY add operator controls (allowed room codes, kicking a vip: CLOSED reason 3).
- The token stops off-path injection into a session; an on-path attacker who reads the traffic can impersonate a
  member until it next sends a KEEPALIVE.

## 8. Recommended relay logging

One line per join (room, vip, name, source address, session id), leave or timeout, and handshake refusal (reason,
source address); counters of datagrams received, sent, forwarded and dropped (per drop reason) every few seconds.
Payloads are never logged.

## 9. Test vectors

Nonce 0123456789ABCDEF, cookie A0 A1 .. AF, session id C0FFEE01, token 11 22 33 44 55 66 77 88, vip 10.13.0.2.
`runtime/lan_probe.c` checks the client's encoder against the JOIN, JOIN_ACK and DATA vectors byte for byte.

    HELLO (64 bytes)
        53 33 4c 52 01 01 00 00 01 23 45 67 89 ab cd ef   then 48 zero bytes
    CHALLENGE (32 bytes)
        53 33 4c 52 01 02 00 00 01 23 45 67 89 ab cd ef
        a0 a1 a2 a3 a4 a5 a6 a7 a8 a9 aa ab ac ad ae af
    JOIN, room "ROOM-1", name "Inkling", any vip (64 bytes)
        53 33 4c 52 01 03 00 00 01 23 45 67 89 ab cd ef
        a0 a1 a2 a3 a4 a5 a6 a7 a8 a9 aa ab ac ad ae af
        00 00 00 00 06 07 00 00 52 4f 4f 4d 2d 31 49 6e
        6b 6c 69 6e 67 00 00 00 00 00 00 00 00 00 00 00
    JOIN_ACK, prefix 16, 12 members, keepalive 5 s, idle 30 s, payload 1440 (40 bytes)
        53 33 4c 52 01 04 00 00 01 23 45 67 89 ab cd ef
        c0 ff ee 01 11 22 33 44 55 66 77 88 0a 0d 00 02
        10 0c 05 1e 05 a0 00 00
    JOIN_NAK, room full (24 bytes)
        53 33 4c 52 01 05 00 00 01 23 45 67 89 ab cd ef
        04 00 00 00 00 00 00 00
    KEEPALIVE, sequence 7 (28 bytes)
        53 33 4c 52 01 06 00 00 c0 ff ee 01 11 22 33 44
        55 66 77 88 00 00 00 07 00 00 00 00
    KEEPALIVE_ACK, sequence 7, 2 members (28 bytes)
        53 33 4c 52 01 07 00 00 c0 ff ee 01 11 22 33 44
        55 66 77 88 00 00 00 07 02 00 00 00
    LEAVE (20 bytes)
        53 33 4c 52 01 08 00 00 c0 ff ee 01 11 22 33 44
        55 66 77 88
    CLOSED, idle timeout (24 bytes)
        53 33 4c 52 01 09 00 00 c0 ff ee 01 11 22 33 44
        55 66 77 88 01 00 00 00
    DATA, 10.13.0.2:49152 to 10.13.255.255:35000, payload de ad be ef (36 bytes)
        53 33 4c 52 01 10 00 00 c0 ff ee 01 11 22 33 44
        55 66 77 88 0a 0d 00 02 c0 00 0a 0d ff ff 88 b8
        de ad be ef
    MEMBER, 10.13.0.3 "Octo" joined (32 bytes)
        53 33 4c 52 01 11 00 00 c0 ff ee 01 11 22 33 44
        55 66 77 88 01 04 00 00 0a 0d 00 03 4f 63 74 6f

## 10. Checking a relay

`splat3.exe --probe-lan-peer` acts as the game's LAN module does (UDP 49152 and 35000, broadcast discovery, unicast
round trips) through the real client, without game files beyond the executable. Start the relay, then in two or more
shells:

    $env:KLM_LAN = 'relay:127.0.0.1:47313'; $env:KLM_LAN_ROOM = 'TEST'; splat3.exe --probe-lan-peer

Each process prints `probe-lan-peer: passed` once it heard another's broadcast and completed three unicast round
trips (exit code 0). It also checks that its own sends to public addresses are refused before they reach the relay.
