#!/usr/bin/python

import datetime
import hashlib
import inspect
import json
import logging
import time
from datetime import timedelta
from typing import Any, Awaitable, Callable, Union

import pytest
import requests
import asyncio
from pyln.testing.fixtures import *  # noqa: F403
from pyln.testing.utils import wait_for, TIMEOUT
from util import get_plugin  # noqa: F401

from nostr_sdk import (
    Client,
    RelayUrl,
    EventBuilder,
    Filter,
    Keys,
    Kind,
    NostrSigner,
    ZapRequestData,
    HandleNotification,
    Event,
    PublicKey,
    NostrWalletConnectUri,
)

LOGGER = logging.getLogger(__name__)


class NotificationHandler(HandleNotification):
    def __init__(self, events_list, stop_after):
        self.events_list = events_list
        self.stop_after = stop_after
        self._done = asyncio.Event()

    async def handle(self, relay_url, subscription_id, event: Event):
        LOGGER.info(f"Received new event from {relay_url}: {event.as_json()}")
        self.events_list.append(event)
        if len(self.events_list) >= self.stop_after:
            self._done.set()

    async def handle_msg(self, relay_url, msg):
        _var = None


Action = Union[
    Callable[[], Awaitable[None]],
    Callable[[], None],
    Awaitable[None],
]


async def fetch_event_responses(
    client: Client,
    client_pubkey: PublicKey,
    event_kind: int,
    action: Action,
    stop_after: int,
    timeout: int = TIMEOUT,
) -> tuple[list[Event], Any]:
    events = []
    response_filter = Filter().kind(Kind(event_kind)).pubkey(client_pubkey)
    await client.subscribe(response_filter)

    handler = NotificationHandler(events, stop_after)
    task = asyncio.create_task(client.handle_notifications(handler))

    time.sleep(1)
    if inspect.iscoroutine(action):
        action_result = await action
    elif inspect.iscoroutinefunction(action):
        action_result = await action()
    elif callable(action):
        action_result = await asyncio.to_thread(action)
    else:
        raise TypeError("action must be a callable or an awaitable")

    try:
        await asyncio.wait_for(handler._done.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        print(
            f"Timeout reached after {timeout} seconds, collected {len(events)} events"
        )
    finally:
        task.cancel()
        try:
            await task
        except asyncio.CancelledError:
            pass

    await client.unsubscribe_all()
    assert len(events) == stop_after
    return (events, action_result)


async def fetch_info_event(
    client: Client,
    uri: NostrWalletConnectUri,
) -> Event:
    response_filter = Filter().kind(Kind(13194)).author(uri.public_key())
    events = await client.fetch_events(
        response_filter, timeout=timedelta(seconds=TIMEOUT)
    )
    start_time = datetime.now()
    while events.len() < 1 and (datetime.now() - start_time) < timedelta(
        seconds=TIMEOUT
    ):
        time.sleep(1)
        events = await client.fetch_events(
            response_filter, timeout=timedelta(seconds=1)
        )
    assert events.len() == 1

    return events.first()


def test_clnaddress(node_factory, get_plugin):  # noqa: F811
    port = node_factory.get_unused_port()
    url = f"localhost:{port}"
    user_name = "testuser"
    l1, l2 = node_factory.line_graph(
        2,
        wait_for_announce=True,
        opts=[
            {"log-level": "debug"},
            {
                "log-level": "debug",
                "plugin": get_plugin,
                "clnaddress-listen": url,
                "clnaddress-base-url": f"http://{url}/",
                "clnaddress-min-receivable": 2,
                "clnaddress-max-receivable": 3000,
            },
        ],
    )
    wait_for(lambda: l2.daemon.is_in_log("Starting lnurlp server."))

    response_lnurl = requests.get(f"http://{url}/lnurlp")
    assert response_lnurl.status_code == 200
    assert json.loads(response_lnurl.json()["metadata"]) == [
        ["text/plain", "Thank you :)"]
    ]

    callback = response_lnurl.json()["callback"]
    assert callback == f"http://{url}/invoice"
    response_invoice = requests.get(callback, params={"amount": 2})
    assert response_invoice.status_code == 200
    assert "pr" in response_invoice.json()
    pay = l1.rpc.call("pay", {"bolt11": response_invoice.json()["pr"]})
    invoice = l2.rpc.call("listinvoices", {"payment_hash": pay["payment_hash"]})[
        "invoices"
    ][0]
    assert invoice["status"] == "paid"
    assert invoice["amount_received_msat"] == 2
    assert json.loads(invoice["description"]) == [["text/plain", "Thank you :)"]]

    result = l2.rpc.call("clnaddress-adduser", [user_name, False, "MONEY, NOW!"])
    assert result["user"] == user_name
    assert result["mode"] == "added"
    assert result["is_email"] is False
    assert result["description"] == "MONEY, NOW!"

    response = requests.get(f"http://{url}/.well-known/lnurlp/{user_name}")
    assert response.status_code == 200

    json_data = response.json()

    assert isinstance(json_data, dict), "Response should be a dictionary"
    assert json.loads(json_data["metadata"]) == [
        ["text/plain", "MONEY, NOW!"],
        ["text/identifier", f"testuser@{url}"],
    ]

    callback = response.json()["callback"]
    assert callback == f"http://{url}/invoice/{user_name}"
    response_invoice = requests.get(callback, params={"amount": 2100})
    assert response_invoice.status_code == 200
    assert "pr" in response_invoice.json()
    pay = l1.rpc.call("pay", {"bolt11": response_invoice.json()["pr"]})
    invoice = l2.rpc.call("listinvoices", {"payment_hash": pay["payment_hash"]})[
        "invoices"
    ][0]
    assert invoice["status"] == "paid"
    assert invoice["amount_received_msat"] == 2100
    assert json.loads(invoice["description"]) == [
        ["text/plain", "MONEY, NOW!"],
        ["text/identifier", f"testuser@{url}"],
    ]
    invoice = l2.rpc.call("decode", [response_invoice.json()["pr"]])
    assert (
        invoice["description_hash"]
        == hashlib.sha256(
            f'[["text/plain","MONEY, NOW!"],["text/identifier","testuser@{url}"]]'.encode()
        ).hexdigest()
    )
    assert invoice["amount_msat"] == 2100

    result = l2.rpc.call("clnaddress-adduser", [user_name, True, "MONEY, LATER!"])
    assert result["user"] == user_name
    assert result["mode"] == "updated"
    assert result["is_email"] is True
    assert result["description"] == "MONEY, LATER!"

    response = requests.get(f"http://{url}/.well-known/lnurlp/{user_name}")
    assert response.status_code == 200

    json_data = response.json()

    assert isinstance(json_data, dict), "Response should be a dictionary"
    assert json.loads(json_data["metadata"]) == [
        ["text/plain", "MONEY, LATER!"],
        ["text/email", f"testuser@{url}"],
    ]

    callback = response.json()["callback"]
    assert callback == f"http://{url}/invoice/{user_name}"
    response_invoice = requests.get(callback, params={"amount": 3000})
    assert response_invoice.status_code == 200
    assert "pr" in response_invoice.json()
    pay = l1.rpc.call("pay", {"bolt11": response_invoice.json()["pr"]})
    invoice = l2.rpc.call("listinvoices", {"payment_hash": pay["payment_hash"]})[
        "invoices"
    ][0]
    assert invoice["status"] == "paid"
    assert invoice["amount_received_msat"] == 3000
    assert json.loads(invoice["description"]) == [
        ["text/plain", "MONEY, LATER!"],
        ["text/email", f"testuser@{url}"],
    ]
    invoice = l1.rpc.call("decode", [response_invoice.json()["pr"]])
    assert (
        invoice["description_hash"]
        == hashlib.sha256(
            f'[["text/plain","MONEY, LATER!"],["text/email","testuser@{url}"]]'.encode()
        ).hexdigest()
    )
    assert invoice["amount_msat"] == 3000

    response_invoice = requests.get(callback, params={"amount": 1})
    assert response_invoice.status_code == 400
    assert response_invoice.json()["reason"] == "`amount` below minimum: 1<2"

    response_invoice = requests.get(callback, params={"amount": 3001})
    assert response_invoice.status_code == 400
    assert response_invoice.json()["reason"] == "`amount` above maximum: 3001>3000"

    response_invoice = requests.get(callback)
    assert response_invoice.status_code == 400

    l2.rpc.call("clnaddress-adduser", [69, False, 42])
    l2.rpc.call("clnaddress-deluser", [69])

    l2.rpc.call("clnaddress-adduser", {"user": 69, "description": 42})
    l2.rpc.call("clnaddress-deluser", {"user": 69})


@pytest.mark.asyncio
async def test_nostr(node_factory, get_plugin, nostr_relay):  # noqa: F811
    relay_url = RelayUrl.parse(nostr_relay)
    port = node_factory.get_unused_port()
    url = f"localhost:{port}"
    user_name = "testuser"
    zapper_keys = Keys.generate()
    l1, l2 = node_factory.line_graph(
        2,
        wait_for_announce=True,
        opts=[
            {"log-level": "debug"},
            {
                "log-level": "debug",
                "plugin": get_plugin,
                "clnaddress-listen": url,
                "clnaddress-base-url": f"http://{url}/",
                "clnaddress-min-receivable": 2,
                "clnaddress-max-receivable": 3000,
                "clnaddress-nostr-privkey": zapper_keys.secret_key().to_hex(),
            },
        ],
    )
    wait_for(lambda: l2.daemon.is_in_log("Starting lnurlp server."))

    l2.rpc.call("clnaddress-adduser", [user_name, False, "MONEY, NOW!"])

    response = requests.get(f"http://{url}/.well-known/lnurlp/{user_name}")
    assert response.status_code == 200

    callback = response.json()["callback"]
    client_keys = Keys.generate()
    receiver_keys = Keys.generate()
    zap_request = EventBuilder.public_zap_request(
        ZapRequestData(receiver_keys.public_key(), [relay_url]).amount(2100)
    ).sign_with_keys(client_keys)
    LOGGER.info(f"python_zap_request:{zap_request.as_json()}")
    response_invoice = requests.get(
        callback, params={"amount": 2100, "nostr": zap_request.as_json()}
    )
    assert response_invoice.status_code == 200
    assert "pr" in response_invoice.json()
    pay = l1.rpc.call("pay", {"bolt11": response_invoice.json()["pr"]})
    invoice = l2.rpc.call("listinvoices", {"payment_hash": pay["payment_hash"]})[
        "invoices"
    ][0]
    assert invoice["status"] == "paid"
    assert invoice["amount_received_msat"] == 2100
    assert json.loads(invoice["description"]) == json.loads(zap_request.as_json())
    invoice = l2.rpc.call("decode", [response_invoice.json()["pr"]])
    assert (
        invoice["description_hash"]
        == hashlib.sha256(zap_request.as_json().encode()).hexdigest()
    )
    assert invoice["amount_msat"] == 2100

    signer = NostrSigner.keys(receiver_keys)
    nostr_client = Client(signer)
    await nostr_client.add_relay(relay_url)
    await nostr_client.connect()

    zap_filter = Filter().kind(Kind(9735))
    events = await nostr_client.fetch_events(zap_filter, timeout=timedelta(seconds=10))
    assert events.len() > 0, "No zap receipts found"
    zap_receipt = json.loads(events.first().as_json())
    LOGGER.info(zap_receipt)
    description_found = False
    for tag in zap_receipt["tags"]:
        if tag[0] == "description":
            description_found = True
            assert json.loads(tag[1]) == json.loads(zap_request.as_json())
    assert description_found
