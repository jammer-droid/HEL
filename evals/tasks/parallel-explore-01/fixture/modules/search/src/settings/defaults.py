"""Default settings for the search service."""

from .base import BASE_TIMEOUT_MS

# Index lookups take longer than the shared base.
TIMEOUT_MS = BASE_TIMEOUT_MS + 350
PAGE_SIZE = 20
