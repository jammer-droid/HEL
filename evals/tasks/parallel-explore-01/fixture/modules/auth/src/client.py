from .settings.defaults import TIMEOUT_MS


def request_options(overrides=None):
    options = {"timeout_ms": TIMEOUT_MS}
    options.update(overrides or {})
    return options
