---
audience: user
ordering_override: -34
---

# Edge Headers

The reverse proxy adds a fixed set of response headers to every response it
serves, whatever the upstream app does. They are applied at the edge because
that is the last hop a webspace cannot override.

| Header | Value | Notes |
|--------|-------|-------|
| `Strict-Transport-Security` | `max-age=63072000; includeSubDomains` | Only on HTTPS responses — browsers ignore it over plain HTTP. No `preload`: that is a zone-level decision. |
| `X-Content-Type-Options` | `nosniff` | |
| `Referrer-Policy` | `strict-origin-when-cross-origin` | |
| `X-Frame-Options` | `DENY` | Only if the app did not set its own. |
| `Content-Security-Policy` | `frame-ancestors 'none'` | Added as an *extra* header. Browsers intersect several CSP headers, so a webspace's own policy keeps working and framing is still blocked. |

`Server` from the upstream is dropped: an app's build string (for example a
container's nginx version) should not be readable from outside.

Plain HTTP is not served at all: every request to a known host is answered with
`301` to the same path and query on `https://` before it reaches an upstream, so
nothing (an update feed's `manifest.json` in particular) can be fetched over the
clear. The `/.well-known/web-agency.json` reachability marker and unknown hosts
are answered directly, since neither has an HTTPS origin to send a visitor to.

An app that already sends a stricter value of its own keeps it for
`X-Frame-Options`, and its CSP is intersected with the edge's `frame-ancestors`
rather than replaced.
