import asyncio
import pytest
import httpx

CASES = [
    ("application/json", b'[{"a":1},', b'2]'),
    ("application/ndjson", b'{"a":1}\n', b'2\n'),
    ("application/json-seq", b'\x1e{"a":1}\n\x1e', b'2\n'),
]

@pytest.mark.parametrize("media,first,second", CASES)
def test_sync_yields_before_requesting_remaining_body(media,first,second):
    released = False
    class Stream(httpx.SyncByteStream):
        def __iter__(self):
            yield first
            assert released, "iterator requested remaining body before yielding complete first value"
            yield second
    response = httpx.Response(200,headers={"content-type":media},stream=Stream())
    values = response.iter_json()
    try:
        assert next(values) == {"a":1}
        released = True
        assert list(values) == [2]
        assert response.is_closed
        with pytest.raises(httpx.StreamConsumed): list(response.iter_json())
    finally:
        response.close()

@pytest.mark.parametrize("media,first,second", CASES)
def test_async_yields_before_requesting_remaining_body(media,first,second):
    async def scenario():
        released = False
        class Stream(httpx.AsyncByteStream):
            async def __aiter__(self):
                yield first
                assert released, "iterator requested remaining body before yielding complete first value"
                yield second
        response = httpx.Response(200,headers={"content-type":media},stream=Stream())
        values = response.aiter_json()
        try:
            assert await anext(values) == {"a":1}
            released = True
            assert [v async for v in values] == [2]
            assert response.is_closed
            with pytest.raises(httpx.StreamConsumed):
                [v async for v in response.aiter_json()]
        finally:
            await response.aclose()
    asyncio.run(scenario())
