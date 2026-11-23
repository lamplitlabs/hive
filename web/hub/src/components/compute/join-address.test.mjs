import assert from "node:assert/strict";
import { test } from "node:test";
import { isLoopbackAddress, isLoopbackHost, joinTunnelLines, parseJoinAddress } from "./join-address.ts";

test("the console's address gives its host and its real port", () => {
  assert.deepEqual(parseJoinAddress("http://127.0.0.1:8765"), { host: "127.0.0.1", port: 8765 });
  assert.deepEqual(parseJoinAddress(" http://localhost:43117/ "), { host: "localhost", port: 43117 });
  assert.deepEqual(parseJoinAddress("http://[::1]:8765"), { host: "[::1]", port: 8765 });
  assert.deepEqual(parseJoinAddress("https://hive.example.com"), { host: "hive.example.com", port: 443 });
  assert.deepEqual(parseJoinAddress("http://10.0.0.5"), { host: "10.0.0.5", port: 80 });
  for (const none of ["", "   ", "127.0.0.1:8765", "ssh://127.0.0.1:22", "not an address"]) assert.equal(parseJoinAddress(none), null, none);
});

test("127.x, localhost and ::1 are this machine; everything else is not", () => {
  for (const host of ["127.0.0.1", "127.0.1.1", "127.255.255.254", "localhost", "LOCALHOST", "localhost.", "hive.localhost", "::1", "[::1]"]) assert.equal(isLoopbackHost(host), true, host);
  for (const host of ["", "128.0.0.1", "10.0.0.5", "192.168.1.20", "0.0.0.0", "127.0.0.256", "1127.0.0.1", "mac-mini.local", "localhost.example.com", "notlocalhost", "::2", "fe80::1", "hive.example.com"]) assert.equal(isLoopbackHost(host), false, host);
  assert.equal(isLoopbackAddress("http://127.0.0.1:8765"), true);
  assert.equal(isLoopbackAddress("http://localhost:8765"), true);
  assert.equal(isLoopbackAddress("http://[::1]:8765"), true);
  assert.equal(isLoopbackAddress("https://hive.example.com"), false);
  assert.equal(isLoopbackAddress("http://192.168.1.20:8765"), false);
  assert.equal(isLoopbackAddress(""), false);
});

test("a loopback address gets the tunnel line with the console's port, then the join line", () => {
  const command = "hive-hand join --brain http://127.0.0.1:8765 amber-river-cobalt-maple-orbit-lantern";
  assert.deepEqual(joinTunnelLines("http://127.0.0.1:8765", command), ["ssh -N -L 8765:127.0.0.1:8765 YOU@THIS-MACHINE &", command]);
  // The port is the console's, not a fixed one, and the host is the one the browser used.
  assert.deepEqual(joinTunnelLines("http://localhost:43117", "hive-hand join --brain http://localhost:43117 a-b-c-d-e-f"), ["ssh -N -L 43117:localhost:43117 YOU@THIS-MACHINE &", "hive-hand join --brain http://localhost:43117 a-b-c-d-e-f"]);
  assert.deepEqual(joinTunnelLines("http://[::1]:8765", " hive-hand join --brain http://[::1]:8765 a-b-c-d-e-f "), ["ssh -N -L 8765:[::1]:8765 YOU@THIS-MACHINE &", "hive-hand join --brain http://[::1]:8765 a-b-c-d-e-f"]);
});

test("an address another machine can reach, or none, gets no tunnel", () => {
  assert.equal(joinTunnelLines("https://hive.example.com", "hive-hand join --brain https://hive.example.com a-b-c-d-e-f"), null);
  assert.equal(joinTunnelLines("http://192.168.1.20:8765", "hive-hand join --brain http://192.168.1.20:8765 a-b-c-d-e-f"), null);
  assert.equal(joinTunnelLines("", "hive-hand join a-b-c-d-e-f"), null);
  assert.equal(joinTunnelLines("http://127.0.0.1:8765", "  "), null);
});
