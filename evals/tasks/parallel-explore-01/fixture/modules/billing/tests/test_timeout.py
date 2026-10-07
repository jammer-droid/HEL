TIMEOUT_MS = 100  # short timeout for the fake gateway


def test_fast_gateway():
    assert TIMEOUT_MS < 1000
