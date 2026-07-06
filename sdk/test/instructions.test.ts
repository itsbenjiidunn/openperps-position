/// Byte-layout tests mirroring the Rust unit tests: the TS encoders must
/// produce exactly the payloads `program/src/instruction.rs` unpacks.

import { test } from "node:test";
import assert from "node:assert/strict";
import { Keypair, PublicKey } from "@solana/web3.js";

import { Tag, encodeOpenPosition, encodeClosePosition } from "../src/instructions.js";
import { positionPda, ata, marketQuoteMint } from "../src/pda.js";

test("encodeOpenPosition layout", () => {
  const d = encodeOpenPosition({
    side: 1,
    assetIndex: 3,
    sizeQ: 1_000_000n,
    execPrice: 123n,
    feeBps: 5n,
    depositAmount: 777n,
    positionBump: 250,
    portfolioBump: 249,
  });
  assert.equal(d.length, 56);
  const dv = new DataView(d.buffer, d.byteOffset);
  assert.equal(d[0], Tag.OpenPosition);
  assert.equal(d[1], 1);
  assert.equal(dv.getUint32(2, true), 3);
  assert.equal(dv.getBigUint64(6, true), 1_000_000n);
  assert.equal(dv.getBigUint64(14, true), 0n);
  assert.equal(dv.getBigUint64(22, true), 123n);
  assert.equal(dv.getBigUint64(30, true), 5n);
  assert.equal(dv.getBigUint64(38, true), 777n);
  assert.equal(d[54], 250);
  assert.equal(d[55], 249);
});

test("encodeClosePosition layout", () => {
  const d = encodeClosePosition({
    side: 0,
    assetIndex: 0,
    sizeQ: 42n,
    execPrice: 100n,
    feeBps: 1n,
    withdrawAmount: 55n,
    positionBump: 248,
  });
  assert.equal(d.length, 55);
  assert.equal(d[0], Tag.ClosePosition);
  assert.equal(d[54], 248);
});

test("PDA derivations are canonical and stable", () => {
  const program = Keypair.generate().publicKey;
  const mint = Keypair.generate().publicKey;

  const [position, bump] = positionPda(program, mint);
  const [position2, bump2] = positionPda(program, mint);
  assert.ok(position.equals(position2));
  assert.equal(bump, bump2);
  assert.ok(!PublicKey.isOnCurve(position.toBytes()));

  const [staging] = ata(position, mint);
  assert.ok(!PublicKey.isOnCurve(staging.toBytes()));
});

test("marketQuoteMint reads the wrapper header and rejects garbage", () => {
  const mint = Keypair.generate().publicKey;
  const data = new Uint8Array(200);
  data.set(new TextEncoder().encode("OPMARKET"), 0);
  new DataView(data.buffer).setUint32(8, 4, true);
  data.set(mint.toBytes(), 48);
  assert.ok(marketQuoteMint(data).equals(mint));

  const stale = new Uint8Array(data);
  new DataView(stale.buffer).setUint32(8, 5, true);
  assert.throws(() => marketQuoteMint(stale));
  assert.throws(() => marketQuoteMint(new Uint8Array(200)));
  assert.throws(() => marketQuoteMint(new Uint8Array(10)));
});
