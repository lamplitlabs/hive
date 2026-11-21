// The address in the join line (Add a machine, Run one command on it) is the
// one the browser used for the console. When that is this machine's own
// loopback address, another machine cannot reach it: it needs an ssh tunnel
// to this machine's console port first. Pure, so node --test can check it
// (join-address.test.mjs).

export interface JoinAddress {
  // The host as the tunnel names it: 127.0.0.1, localhost, or [::1] in brackets.
  host: string;
  // The console's port; the scheme's own when the address names none.
  port: number;
}

// The host and port of a console address such as http://127.0.0.1:8765, or
// null when it is not one (the hub sends "" when it could not tell).
export function parseJoinAddress(address: string): JoinAddress | null {
  let url: URL;
  try {
    url = new URL((address || "").trim());
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  if (!url.hostname) return null;
  const port = url.port ? Number(url.port) : url.protocol === "https:" ? 443 : 80;
  return { host: url.hostname, port };
}

// 127.0.0.0/8, localhost (and names under it) and ::1 reach only the machine
// they are typed on.
export function isLoopbackHost(host: string): boolean {
  const h = (host || "").trim().toLowerCase().replace(/^\[|\]$/g, "").replace(/\.$/, "");
  if (h === "localhost" || h.endsWith(".localhost")) return true;
  if (h === "::1" || h === "0:0:0:0:0:0:0:1") return true;
  const v4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(h);
  return !!v4 && v4[1] === "127" && v4.slice(1).every((n) => Number(n) <= 255);
}

export function isLoopbackAddress(address: string): boolean {
  const a = parseJoinAddress(address);
  return !!a && isLoopbackHost(a.host);
}

// What stands for the reader's own login on this machine in the tunnel line.
export const TUNNEL_LOGIN = "YOU@THIS-MACHINE";

// The two lines to run on the other machine when the console's address is a
// loopback one: the tunnel (its port on that machine forwarded to the same
// port here), then the join line as the console gave it, which now reaches
// this machine through the tunnel. null when the address is not loopback, or
// is unknown: the join line works as it is, or there is nothing to build on.
export function joinTunnelLines(address: string, command: string): [string, string] | null {
  const a = parseJoinAddress(address);
  if (!a || !isLoopbackHost(a.host) || !command.trim()) return null;
  return ["ssh -N -L " + a.port + ":" + a.host + ":" + a.port + " " + TUNNEL_LOGIN + " &", command.trim()];
}
