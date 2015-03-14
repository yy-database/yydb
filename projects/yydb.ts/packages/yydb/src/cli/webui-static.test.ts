import assert from 'node:assert/strict';
import { test } from 'node:test';

import { wsEndpointFromBind } from './webui-static.ts';

test('wsEndpointFromBind maps loopback HTTP bind to wire WebSocket URL', () => {
    assert.equal(wsEndpointFromBind('127.0.0.1:7700'), 'ws://127.0.0.1:7700/wire');
});

test('wsEndpointFromBind preserves custom path when wire suffix is present', () => {
    assert.equal(
        wsEndpointFromBind('127.0.0.1:7700/custom/wire'),
        'ws://127.0.0.1:7700/custom/wire',
    );
});

test('wsEndpointFromBind upgrades https bind to wss', () => {
    assert.equal(wsEndpointFromBind('https://db.example:443'), 'wss://db.example/wire');
});
