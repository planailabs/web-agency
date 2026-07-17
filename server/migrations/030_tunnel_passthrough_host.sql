-- Tunnel folders can optionally pass the visitor's Host header through to the
-- upstream instead of rewriting it to the tunnel URL's hostname (the default).
ALTER TABLE webspaces ADD COLUMN tunnel_passthrough_host BOOLEAN NOT NULL DEFAULT false;
