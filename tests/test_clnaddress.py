import asyncio
import datetime
import hashlib
import inspect
import json
import logging
import uuid
from collections.abc import Awaitable, Callable
from datetime import timedelta
from typing import Any, Union

import pytest
import requests
from nostr_sdk import (
    Client,
    Event,
    EventBuilder,
    Filter,
    Keys,
    Kind,
    NostrWalletConnectUri,
    PublicKey,
    RelayUrl,
    ReqTarget,
    Tag,
)
from pyln.testing.fixtures import *
from pyln.testing.utils import TIMEOUT, wait_for
from util import get_plugin  # noqa: F401

LOGGER = logging.getLogger(__name__)


Action = Union[  # noqa: UP007
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
    target = ReqTarget.auto([response_filter])

    subscription_id = uuid.uuid4().hex
    LOGGER.info(f"Subscribing with id {subscription_id} to {response_filter}")

    await client.subscribe(target, subscription_id)

    async def collect_events():
        stream = client.notifications()

        while len(events) < stop_after:
            notification = await stream.next()

            if notification.is_new_event():
                event = notification.event
                relay_url = notification.relay_url

                LOGGER.info(f"Received new event from {relay_url}: {event.as_json()}")

                events.append(event)

    task = asyncio.create_task(collect_events())

    await asyncio.sleep(1)

    if inspect.iscoroutine(action):
        action_result = await action
    elif inspect.iscoroutinefunction(action):
        action_result = await action()
    elif callable(action):
        action_result = await asyncio.to_thread(action)
    else:
        raise TypeError("action must be a callable or an awaitable")

    try:
        await asyncio.wait_for(task, timeout=timeout)
    except asyncio.TimeoutError:
        print(
            f"Timeout reached after {timeout} seconds, collected {len(events)} events",
        )
    finally:
        task.cancel()
        try:
            await task
        except asyncio.CancelledError:
            pass

        await client.unsubscribe_all()

    assert len(events) == stop_after
    return events, action_result


async def fetch_info_event(
    client: Client,
    uri: NostrWalletConnectUri,
) -> Event:
    response_filter = Filter().kind(Kind(13194)).author(uri.public_key())
    target = ReqTarget.auto([response_filter])
    events = await client.fetch_events(target, timeout=timedelta(seconds=TIMEOUT))
    start_time = datetime.now()
    while len(events) < 1 and (datetime.now() - start_time) < timedelta(
        seconds=TIMEOUT
    ):
        await asyncio.sleep(1)
        events = await client.fetch_events(target, timeout=timedelta(seconds=1))
    assert len(events) == 1

    return events[0]


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
    invstring = response_invoice.json()["pr"]
    l1.rpc.call("xpay", {"invstring": invstring})
    invoice = l2.rpc.call("listinvoices", {"invstring": invstring})["invoices"][0]
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
    invstring = response_invoice.json()["pr"]
    l1.rpc.call("xpay", {"invstring": invstring})
    invoice = l2.rpc.call("listinvoices", {"invstring": invstring})["invoices"][0]
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
    invstring = response_invoice.json()["pr"]
    l1.rpc.call("xpay", {"invstring": invstring})
    invoice = l2.rpc.call("listinvoices", {"invstring": invstring})["invoices"][0]
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
    assert response_invoice.status_code == 200
    assert response_invoice.json()["reason"] == "`amount` below minimum: 1<2"

    response_invoice = requests.get(callback, params={"amount": 3001})
    assert response_invoice.status_code == 200
    assert response_invoice.json()["reason"] == "`amount` above maximum: 3001>3000"

    response_invoice = requests.get(callback)
    assert response_invoice.status_code == 400

    l2.rpc.call("clnaddress-adduser", [69, False, 42])
    l2.rpc.call("clnaddress-deluser", [69])

    l2.rpc.call("clnaddress-adduser", {"user": 69, "description": 42})
    l2.rpc.call("clnaddress-deluser", {"user": 69})


def test_nostr_key_file(tmp_path, node_factory, get_plugin):  # noqa: F811
    port = node_factory.get_unused_port()
    url = f"localhost:{port}"
    zapper_keys = Keys.generate()
    key_file = tmp_path / "nostr-key"
    key_file.write_text(zapper_keys.secret_key().to_hex())
    l2 = node_factory.get_node(
        options={
            "log-level": "debug",
            "plugin": get_plugin,
            "clnaddress-listen": url,
            "clnaddress-base-url": f"http://{url}/",
            "clnaddress-nostr-privkey-file": str(key_file),
        },
    )
    wait_for(lambda: l2.daemon.is_in_log("Starting lnurlp server."))

    response = requests.get(f"http://{url}/lnurlp")
    assert response.status_code == 200
    assert response.json()["nostrPubkey"] == zapper_keys.public_key().to_hex()


def test_nostr_key_migration(node_factory, get_plugin):  # noqa: F811
    port = node_factory.get_unused_port()
    url = f"localhost:{port}"
    zapper_keys = Keys.generate()
    l1 = node_factory.get_node(
        options={
            "log-level": "debug",
            "plugin": get_plugin,
            "clnaddress-listen": url,
            "clnaddress-base-url": f"http://{url}/",
            "clnaddress-nostr-privkey": zapper_keys.secret_key().to_hex(),
        }
    )
    wait_for(lambda: l1.daemon.is_in_log("Starting lnurlp server."))

    network = l1.rpc.getinfo()["network"]
    key_file = l1.lightning_dir / network / "clnaddress" / "nostr-secret-key"
    assert key_file.read_text().strip() == zapper_keys.secret_key().to_hex()
    assert (key_file.stat().st_mode & 0o777) == 0o600

    response = requests.get(f"http://{url}/lnurlp")
    assert response.status_code == 200
    assert response.json()["nostrPubkey"] == zapper_keys.public_key().to_hex()

    # Removing the legacy option keeps zap working via the migrated file.
    l1.daemon.opts.pop("clnaddress-nostr-privkey", None)
    l1.restart()
    wait_for(lambda: l1.daemon.is_in_log("Starting lnurlp server."))

    response = requests.get(f"http://{url}/lnurlp")
    assert response.status_code == 200
    assert response.json()["nostrPubkey"] == zapper_keys.public_key().to_hex()


@pytest.mark.asyncio
async def test_nostr(nostr_relay, node_factory, get_plugin):  # noqa: F811
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
                "broken_log": r"Relay receiver exited with error",
            },
        ],
    )
    wait_for(lambda: l2.daemon.is_in_log("Starting lnurlp server."))

    l2.rpc.call("clnaddress-adduser", [user_name, False, "MONEY, NOW!"])

    response = requests.get(f"http://{url}/.well-known/lnurlp/{user_name}")  # noqa: ASYNC210
    assert response.status_code == 200
    nostr_pubkey = response.json()["nostrPubkey"]

    callback = response.json()["callback"]
    client_keys = Keys.generate()
    receiver_keys = Keys.generate()
    zap_request = build_zap_request(
        client_keys,
        receiver_keys.public_key(),
        [str(relay_url)],
        nostr_key=nostr_pubkey,
        amount_msats=2100,
    )
    LOGGER.info(f"python_zap_request:{zap_request.as_json()}")
    response_invoice = requests.get(  # noqa: ASYNC210
        callback, params={"amount": 2100, "nostr": zap_request.as_json()}
    )
    assert response_invoice.status_code == 200
    assert "pr" in response_invoice.json()
    invstring = response_invoice.json()["pr"]
    l1.rpc.call("xpay", {"invstring": invstring})
    invoice = l2.rpc.call("listinvoices", {"invstring": invstring})["invoices"][0]
    assert invoice["status"] == "paid"
    assert invoice["amount_received_msat"] == 2100
    assert json.loads(invoice["description"]) == json.loads(zap_request.as_json())
    invoice = l2.rpc.call("decode", [response_invoice.json()["pr"]])
    assert (
        invoice["description_hash"]
        == hashlib.sha256(zap_request.as_json().encode()).hexdigest()
    )
    assert invoice["amount_msat"] == 2100

    nostr_client = Client()
    await nostr_client.add_relay(relay_url)
    await nostr_client.connect()

    zap_filter = Filter().kind(Kind(9735))
    req_target = ReqTarget.auto([zap_filter])
    events = await nostr_client.fetch_events(req_target, timeout=timedelta(seconds=10))
    assert len(events) > 0, "No zap receipts found"
    zap_receipt = json.loads(events[0].as_json())
    LOGGER.info(zap_receipt)
    description_found = False
    for tag in zap_receipt["tags"]:
        if tag[0] == "description":
            description_found = True
            assert json.loads(tag[1]) == json.loads(zap_request.as_json())
    assert description_found


def build_zap_request(
    sender_keys, recipient_pubkey, relays, amount_msats=None, nostr_key=None, message=""
):
    tags = [
        Tag.public_key(recipient_pubkey),
        Tag.custom("relays", relays),
    ]
    if nostr_key:
        tags.append(Tag.custom("P", [nostr_key]))
    if amount_msats:
        tags.append(Tag.custom("amount", [str(amount_msats)]))

    builder = EventBuilder(Kind(9734), message).tags(tags)
    return builder.finalize(sender_keys)
