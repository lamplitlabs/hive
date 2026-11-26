"use client";

import { useEffect } from "react";
import { SkeletonPage } from "@/components/hive/panel";

// The console opens on Sessions (TASK-129: Home and For you went). The
// export is static, so "/" forwards in the browser to /chat/, keeping the
// query and the hash (an old "/#for-you" link lands on the hard-stop strip).
// What Home showed and is still needed sits on Sessions: the broken-strip
// and the hard-stop asks over the page, the set-up card and the agents at
// work under the new-session box.
export default function HomePage() {
  useEffect(() => {
    const hash = window.location.hash === "#for-you" ? "#hard-stops" : window.location.hash;
    window.location.replace("/chat/" + window.location.search + hash);
  }, []);
  return <SkeletonPage rows={3} />;
}
