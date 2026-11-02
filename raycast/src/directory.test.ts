import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { callLink, readDirectory, teammateForQuicklink, type Teammate } from "./directory.ts";

const person = (id: string, first_name: string, last_name: string, is_active = true): Teammate => ({
  id,
  first_name,
  last_name,
  is_active,
});
const team = [person("1", "Alex", "Rivera"), person("2", "Alexa", "Stone"), person("3", "Mina", "Ono", false)];

const TOKEN = "saved-token";
const ringsFromQuicklink = (name: string, teammates = team) => teammateForQuicklink(teammates, name, TOKEN, TOKEN);

test("a quicklink whose name can only be one online person rings them", () => {
  assert.equal(ringsFromQuicklink("rivera")?.id, "1");
  assert.equal(ringsFromQuicklink("ALEX RIV")?.id, "1");
  assert.equal(ringsFromQuicklink("  stone ")?.id, "2");
});

test("a deeplink without the saved token rings nobody, even with a perfect name", () => {
  assert.equal(teammateForQuicklink(team, "rivera", undefined, TOKEN), null);
  assert.equal(teammateForQuicklink(team, "rivera", "guessed", TOKEN), null);
  assert.equal(teammateForQuicklink(team, "rivera", "", ""), null);
});

test("an ambiguous, unknown, empty or offline name rings nobody", () => {
  assert.equal(ringsFromQuicklink("alex"), null);
  assert.equal(ringsFromQuicklink("nobody"), null);
  assert.equal(ringsFromQuicklink("   "), null);
  assert.equal(ringsFromQuicklink("mina"), null);
});

test("a missing is_active counts as offline", () => {
  const { is_active: _ignored, ...unknownPresence } = person("4", "Sam", "Lee");
  assert.equal(ringsFromQuicklink("sam", [unknownPresence]), null);
});

test("the call link escapes what it carries", () => {
  const link = new URL(callLink(person("a&b=c", "A", "B"), "k+y/1"));
  assert.equal(link.protocol, "hopp:");
  assert.equal(link.pathname, "/call");
  assert.equal(link.searchParams.get("userId"), "a&b=c");
  assert.equal(link.searchParams.get("key"), "k+y/1");
});

test("a missing, corrupt or wrongly shaped file reads as no directory", async () => {
  const folder = await mkdtemp(join(tmpdir(), "hopp-directory-"));
  const file = join(folder, "launcher.json");
  assert.equal(await readDirectory(file), null);
  for (const contents of ["{not json", "null", "{}", '{"key":1,"teammates":[]}', '{"key":"k","teammates":{}}']) {
    await writeFile(file, contents);
    assert.equal(await readDirectory(file), null, contents);
  }
  await writeFile(file, '{"key":"k","teammates":[]}');
  assert.deepEqual(await readDirectory(file), { key: "k", teammates: [] });
  await rm(folder, { recursive: true });
});
