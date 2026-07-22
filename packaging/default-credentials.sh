# Shared default OAuth2 credentials, sourced by both install.sh and
# packaging/build-packages.sh so there is exactly one place to update them.
#
# Baked in so end users never need their own Google Cloud project — just
# login and grant access. A Desktop-app client_secret isn't a real secret
# (Google's own guidance, RFC 8252); the actual security boundary is the
# per-user token in the OS keyring, not this value. Override via
# TUXDRIVE_CLIENT_ID/TUXDRIVE_CLIENT_SECRET env vars or an [auth] section in
# config.toml if you want your own project.
DEFAULT_CLIENT_ID="1010225147517-unmr815v5sgt8i1k85ulg664dc0rgegn.apps.googleusercontent.com"
DEFAULT_CLIENT_SECRET="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"
