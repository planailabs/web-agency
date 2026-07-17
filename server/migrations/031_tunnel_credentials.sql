-- Tunnel folders can present a TLS client certificate and/or inject a basic
-- auth Authorization header when proxying to the tunnel upstream. Both point
-- at rows in the credentials table (types 'client_cert' and 'basic_auth').
ALTER TABLE webspaces
    ADD COLUMN tunnel_client_cert_credential_id UUID REFERENCES credentials(id) ON DELETE SET NULL,
    ADD COLUMN tunnel_basic_auth_credential_id  UUID REFERENCES credentials(id) ON DELETE SET NULL;
