-- Per-folder control over the headers the edge adds to every response.
-- security_headers off drops them all (HSTS, nosniff, Referrer-Policy and the
-- framing block); frame_ancestors names who may frame the folder instead of
-- nobody (a CSP source list, e.g. "https://cms.example.com 'self'").
ALTER TABLE webspaces ADD COLUMN security_headers BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE webspaces ADD COLUMN frame_ancestors TEXT;
