import asyncio
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from aiohttp import web
from aiohttp.test_utils import TestClient, TestServer
from jev_decider.server import create_app
from jev_decider.settings import Settings
from tests.decision_cases import request, answers

class SettingsTests(unittest.TestCase):
    def test_key_file_and_context_bytes_are_deployment_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            key = Path(directory) / 'key'; key.write_text('fixture-key')
            with patch.dict(os.environ, {'OPENROUTER_API_KEY_FILE': str(key), 'JEV_REQUEST_TIMEOUT_SECONDS': '15'}, clear=True):
                value = Settings.from_env()
                self.assertEqual(value.request_timeout_seconds, 15)
                self.assertEqual(value.max_request_bytes, 256 * 1024)
            with patch.dict(os.environ, {'OPENROUTER_API_KEY_FILE': str(key), 'JEV_REQUEST_TIMEOUT_SECONDS': 'nan'}, clear=True):
                with self.assertRaises(RuntimeError): Settings.from_env()

class HttpFixture(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.calls = []; self.delay = 0; self.status = 200; self.body = None; self.reply = answers()
        async def upstream(req):
            self.calls.append((req.headers.get('Authorization'), await req.json()))
            await asyncio.sleep(self.delay)
            return web.Response(body=self.body, status=self.status) if self.body is not None else web.json_response(self.reply, status=self.status)
        app = web.Application(); app.router.add_post('/decide', upstream)
        self.upstream = TestServer(app); await self.upstream.start_server()
        self.client = TestClient(TestServer(create_app(Settings(api_key='fixture-key', upstream_url=str(self.upstream.make_url('/decide')), model='jev-test', request_timeout_seconds=.25, max_request_bytes=256*1024, max_concurrency=1, inbound_header_name='X-Fixture-Auth', inbound_header_value='fixture-token'))))
        await self.client.start_server()
    async def asyncTearDown(self):
        await self.client.close(); await self.upstream.close()
    async def post(self, value=None, **kwargs):
        return await self.client.post('/v1/decisions', json=request() if value is None else value, headers={'X-Fixture-Auth': 'fixture-token'}, **kwargs)
    async def test_real_handler_forwards_exact_definition_once(self):
        response = await self.post()
        self.assertEqual(response.status, 200)
        self.assertEqual((await response.json())['decision']['probabilities']['simple'], .8)
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(self.calls[0][0], 'Bearer fixture-key')
        self.assertEqual(self.calls[0][1]['questions']['q0']['criteria'], [x['criterion'] for x in request()['decision']['levels']])
    async def test_assessment_and_category_use_one_call(self):
        self.reply = answers(True, 0)
        response = await self.post(request(1))
        body = await response.json()
        self.assertEqual(response.status, 200)
        self.assertEqual(body['decision']['choice'], 'review')
        self.assertEqual(body['assessment']['score'], 0)
        self.assertEqual(len(self.calls), 1)
    async def test_authentication_and_old_contract_fail_before_upstream(self):
        response = await self.client.post('/v1/decisions', json=request())
        self.assertEqual(response.status, 401)
        value = request(); value['branches'] = {}
        self.assertEqual((await self.post(value)).status, 400)
        self.assertEqual(self.calls, [])
    async def test_upstream_status_duplicate_fields_and_size_are_bounded(self):
        self.status = 302
        self.assertEqual((await self.post()).status, 502)
        self.status = 200; self.body = b'{"answers":{},"answers":{}}'
        self.assertEqual((await self.post()).status, 502)
        self.body = b'x' * (64 * 1024 + 1)
        self.assertEqual((await self.post()).status, 502)
    async def test_timeout_including_queue_is_a_visible_failure(self):
        self.delay = .4
        responses = await asyncio.gather(self.post(), self.post())
        self.assertEqual([response.status for response in responses], [504, 504])
