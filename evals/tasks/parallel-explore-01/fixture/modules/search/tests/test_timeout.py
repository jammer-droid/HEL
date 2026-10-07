def test_slow_index(monkeypatch):
    monkeypatch.setattr("search.src.settings.defaults.TIMEOUT_MS", 6000)
