# 0004: A subset of RESP2, the Redis protocol

- **Status:** Accepted
- **Date:** 2026-11-21

## Problem

Clients need a way to talk to the server over TCP. The protocol decides which tools work out of the
box, how hard parsing is to get right, and what a hostile client can do.

## Options

1. **A custom text protocol**, one command per line. Easy to write, but no existing client works with
   it, and values containing line breaks need escaping.
2. **HTTP with JSON.** Familiar and browser-friendly, but much more to parse and to defend, and binary
   values need encoding.
3. **RESP2**, the Redis protocol. Length-prefixed, so binary values pass through untouched; simple to
   parse incrementally; and `redis-cli` and every Redis client library already speak it.
4. **RESP3**, the newer Redis protocol. More reply types than EmberKV needs.

## Decision

Option 3: a subset of RESP2.

- **Requests:** arrays of bulk strings, as clients send, plus inline commands (one line of words) for
  `nc` and `telnet`.
- **Replies:** simple strings, errors, integers, bulk strings and null.
- **Commands:** `PING`, `GET`, `SET`, `DEL`, `EXISTS`. `HELLO` replies with a `NOPROTO` error so that
  `redis-cli` falls back from RESP3 to RESP2.
- **Every count and length is checked against limits before anything is allocated.** A malformed
  request gets an error reply and the connection is closed, since command boundaries can no longer be
  trusted.
- **Inline requests beginning with `POST` or `Host:` are refused** and the connection closed: they come
  from a web browser, possibly driven by a malicious page (cross-protocol scripting). Redis does the
  same. This was found by opening the server in a browser during slice 5.
- **Error text is cleaned of line breaks**, so text a client sent cannot end a reply early and forge a
  second one.

## Consequences

- **Easier:** demos and manual testing with the standard `redis-cli`; binary-safe values; parsing that
  handles requests split across reads and several requests in one read.
- **Harder:** no authentication. The server listens on `127.0.0.1` only, and must stay that way until
  authentication exists.
- **Would change the decision:** a need for clients that cannot speak RESP, or for features only RESP3
  offers, such as server-pushed messages.
