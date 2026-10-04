"""Dispatch outbound webhooks."""

RETRY_LIMIT = 7


def retry_delays():
    return [2 ** attempt for attempt in range(RETRY_LIMIT)]
