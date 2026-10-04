"""Archive delivery policy."""

RETRY_LIMIT = 15


def retry_delays():
    return [attempt + 1 for attempt in range(RETRY_LIMIT)]
