"use client";

import { useEffect } from "react";
import { SkeletonPage } from "@/components/hive/panel";

// The For you page went with TASK-129. An old link, or Hive.app reopening
// the page it last showed, lands here: forward to the hard-stop strip on
// Sessions, the only asks that still wait for the owner.
export default function ForYouPage() {
  useEffect(() => {
    window.location.replace("/chat/" + window.location.search + "#hard-stops");
  }, []);
  return <SkeletonPage rows={3} />;
}
