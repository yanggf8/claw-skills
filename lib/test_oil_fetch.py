"""Unit tests for lib/oil_fetch.py."""
import io
import json
import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import oil_fetch  # noqa: E402


class FakeResponse:
    def __init__(self, payload: dict):
        self.payload = payload

    def __enter__(self):
        return io.StringIO(json.dumps(self.payload))

    def __exit__(self, *exc):
        return False


class OilFetchTests(unittest.TestCase):
    def test_build_chart_url_encodes_symbol(self):
        url = oil_fetch.build_chart_url("CL=F", range_name="1y")
        self.assertIn("CL%3DF", url)
        self.assertIn("range=1y", url)

    def test_parse_chart_response_skips_null_close(self):
        payload = {
            "chart": {
                "result": [
                    {
                        "timestamp": [1713139200, 1713225600, 1713312000],
                        "indicators": {
                            "quote": [
                                {
                                    "close": [82.2, None, 83.4],
                                }
                            ]
                        },
                    }
                ]
            }
        }
        rows = oil_fetch.parse_chart_response(payload)
        self.assertEqual(
            rows,
            [("2024-04-15", 82.2), ("2024-04-17", 83.4)],
        )

    def test_fetch_latest_returns_last_non_null_row(self):
        payload = {
            "chart": {
                "result": [
                    {
                        "timestamp": [1713139200, 1713225600, 1713312000],
                        "indicators": {
                            "quote": [
                                {
                                    "close": [82.2, None, 83.4],
                                }
                            ]
                        },
                    }
                ]
            }
        }
        with mock.patch("urllib.request.urlopen", return_value=FakeResponse(payload)):
            row = oil_fetch.fetch_latest("CL=F")
        self.assertEqual(row, ("2024-04-17", 83.4))


if __name__ == "__main__":
    unittest.main()
