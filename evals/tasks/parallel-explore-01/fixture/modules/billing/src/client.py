from .settings.defaults import TIMEOUT_MS

# Kept only for the retired v1 API; not used by the current client.
LEGACY_TIMEOUT_MS = 9000


def request_options():
    return {"timeout_ms": TIMEOUT_MS}
