import {
  Action,
  ActionPanel,
  Color,
  closeMainWindow,
  Icon,
  Image,
  LaunchProps,
  List,
  LocalStorage,
  open,
  showToast,
  Toast,
} from "@raycast/api";
import { createDeeplink, getAvatarIcon } from "@raycast/utils";
import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import { useEffect, useState } from "react";
import { callLink, Directory, fullName, readDirectory, Teammate, teammateForQuicklink } from "./directory";

const HOPP_APP_PATH = "/Applications/hopp.app";
const QUICKLINK_TOKEN_STORAGE = "quicklink-token";

const loadQuicklinkToken = async () => {
  const saved = await LocalStorage.getItem<string>(QUICKLINK_TOKEN_STORAGE);
  if (saved) return saved;
  const token = randomUUID();
  await LocalStorage.setItem(QUICKLINK_TOKEN_STORAGE, token);
  return token;
};

const isHoppRunning = () =>
  new Promise<boolean>((resolve) => execFile("/usr/bin/pgrep", ["-x", "hopp"], (error) => resolve(!error)));

const callTeammate = async (teammate: Teammate, key: string) => {
  // Opening the link would start Hopp, but Hopp only hears links once it's up, so this call would be lost
  if (!(await isHoppRunning())) {
    await showToast({
      style: Toast.Style.Failure,
      title: "Hopp isn't running",
      primaryAction: { title: "Open Hopp", onAction: () => open(HOPP_APP_PATH) },
    });
    return;
  }
  await closeMainWindow();
  await open(callLink(teammate, key));
};

const TeammateItem = ({
  teammate,
  directoryKey,
  quicklinkToken,
}: {
  teammate: Teammate;
  directoryKey: string;
  quicklinkToken: string;
}) => {
  const name = fullName(teammate);
  return (
    <List.Item
      title={name}
      icon={teammate.avatar_url ? { source: teammate.avatar_url, mask: Image.Mask.Circle } : getAvatarIcon(name)}
      accessories={[
        teammate.is_active ?
          { icon: { source: Icon.CircleFilled, tintColor: Color.Green }, text: "Online" }
        : { icon: { source: Icon.Circle, tintColor: Color.SecondaryText }, text: "Offline" },
      ]}
      actions={
        <ActionPanel>
          <Action
            title={`Call ${teammate.first_name}`}
            icon={Icon.Phone}
            onAction={() => callTeammate(teammate, directoryKey)}
          />
          <Action.CreateQuicklink
            title={`Add ${teammate.first_name} to Raycast Search`}
            icon={Icon.MagnifyingGlass}
            shortcut={{ modifiers: ["cmd"], key: "s" }}
            quicklink={{
              name: `Call ${name}`,
              // Goes through this command, so the call key stays out of Raycast's saved quicklinks
              link: createDeeplink({ command: "call", arguments: { name }, context: { quicklinkToken } }),
            }}
          />
        </ActionPanel>
      }
    />
  );
};

export default function Command(
  props: LaunchProps<{ arguments: Arguments.Call; launchContext: { quicklinkToken?: string } }>,
) {
  const nameOnOpening = (props.fallbackText ?? props.arguments.name ?? "").trim();
  const [searchText, setSearchText] = useState(nameOnOpening);
  const [loaded, setLoaded] = useState<{ directory: Directory | null; quicklinkToken: string }>();

  useEffect(() => {
    Promise.all([readDirectory(), loadQuicklinkToken()]).then(([directory, quicklinkToken]) => {
      setLoaded({ directory, quicklinkToken });
      if (!directory) return;
      const linkToken = props.launchContext?.quicklinkToken;
      const teammate = teammateForQuicklink(directory.teammates, nameOnOpening, linkToken, quicklinkToken);
      if (teammate) callTeammate(teammate, directory.key);
    });
  }, []);

  const teammates = loaded?.directory?.teammates ?? [];
  const online = teammates.filter((teammate) => teammate.is_active);
  const offline = teammates.filter((teammate) => !teammate.is_active);
  const rowProps = { directoryKey: loaded?.directory?.key ?? "", quicklinkToken: loaded?.quicklinkToken ?? "" };

  return (
    <List
      isLoading={!loaded}
      searchBarPlaceholder="Type a name, press return to call"
      searchText={searchText}
      onSearchTextChange={setSearchText}
      filtering={{ keepSectionOrder: true }}
    >
      {loaded && teammates.length === 0 ?
        <List.EmptyView
          icon={Icon.PersonCircle}
          title="Hopp hasn't shared your team yet"
          description="Open Hopp, check you're signed in, then try again."
          actions={
            <ActionPanel>
              <Action title="Open Hopp" icon={Icon.AppWindow} onAction={() => open(HOPP_APP_PATH)} />
            </ActionPanel>
          }
        />
      : <List.EmptyView icon={Icon.MagnifyingGlass} title="No teammate by that name" />}
      <List.Section title="Online">
        {online.map((teammate) => (
          <TeammateItem key={teammate.id} teammate={teammate} {...rowProps} />
        ))}
      </List.Section>
      <List.Section title="Offline">
        {offline.map((teammate) => (
          <TeammateItem key={teammate.id} teammate={teammate} {...rowProps} />
        ))}
      </List.Section>
    </List>
  );
}
