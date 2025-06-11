#!/usr/bin/python

import hashlib
import importlib.resources as pkg_resources
import json
import logging
import socket
import subprocess
import sys
import time
from datetime import timedelta
from threading import Thread

import pytest
import pytest_asyncio
import requests
import yaml
from pyln.testing.fixtures import *  # noqa: F403
from pyln.testing.utils import wait_for
from util import get_plugin  # noqa: F401

if sys.version_info >= (3, 9):
    from nostr_sdk import (
        Client,
        EventBuilder,
        Filter,
        Keys,
        Kind,
        NostrSigner,
        ZapRequestData,
    )

LOGGER = logging.getLogger(__name__)


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


@pytest.mark.skipif(sys.version_info < (3, 9), reason="Requires Python 3.9 or higher")
@pytest.mark.asyncio
async def test_nostr(node_factory, get_plugin, nostr_client):  # noqa: F811
    nostr_client, relay_port = nostr_client
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
        ZapRequestData(
            receiver_keys.public_key(), [f"ws://127.0.0.1:{relay_port}"]
        ).amount(2100)
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

    zap_filter = Filter().kind(Kind(9735))
    events = await nostr_client.fetch_events(zap_filter, timeout=timedelta(seconds=5))
    assert events.len() > 0, "No zap receipts found"
    zap_receipt = json.loads(events.first().as_json())
    LOGGER.info(zap_receipt)
    description_found = False
    for tag in zap_receipt["tags"]:
        if tag[0] == "description":
            description_found = True
            assert json.loads(tag[1]) == json.loads(zap_request.as_json())
    assert description_found


@pytest_asyncio.fixture(scope="function")
async def nostr_client(nostr_relay):
    port = nostr_relay
    keys = Keys.generate()
    signer = NostrSigner.keys(keys)

    client = Client(signer)

    relay_url = f"ws://127.0.0.1:{port}"
    await client.add_relay(relay_url)
    await client.connect()

    yield client, port

    await client.disconnect()


@pytest_asyncio.fixture(scope="function")
async def nostr_relay():
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    dynamic_port = s.getsockname()[1]
    s.close()

    try:
        config_file = pkg_resources.files("nostr_relay").joinpath("config.yaml")
    except KeyError:
        raise FileNotFoundError("config.yaml not found in the nostr package")

    with open(config_file, "r") as file:
        config = yaml.safe_load(file)

    config["gunicorn"]["bind"] = f"127.0.0.1:{dynamic_port}"
    config["authentication"]["valid_urls"] = [
        f"ws://localhost:{dynamic_port}",
        f"ws://127.0.0.1:{dynamic_port}",
    ]

    with open(config_file, "w") as file:
        yaml.safe_dump(config, file)

    LOGGER.info(f"{config_file}")
    process = subprocess.Popen(
        ["nostr-relay", "-c", config_file, "serve"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    stdout_thread = Thread(target=log_pipe, args=(process.stdout, LOGGER, logging.INFO))
    stderr_thread = Thread(
        target=log_pipe, args=(process.stderr, LOGGER, logging.ERROR)
    )
    stdout_thread.start()
    stderr_thread.start()

    time.sleep(2)

    yield dynamic_port

    process.terminate()
    process.wait()

    stdout_thread.join()
    stderr_thread.join()


def log_pipe(pipe, logger, log_level):
    while True:
        line = pipe.readline()
        if not line:
            break
        logger.log(log_level, line.strip())
