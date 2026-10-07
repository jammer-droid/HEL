from .settings.defaults import TIMEOUT_MS, PAGE_SIZE


def request_options(page=1):
    return {"timeout_ms": TIMEOUT_MS, "page": page, "size": PAGE_SIZE}
