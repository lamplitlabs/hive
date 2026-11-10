"use client";

import { Suspense } from "react";
import { ForYou } from "@/components/for-you/for-you";
import { SkeletonPage } from "@/components/hive/panel";

// /for-you/: everything that waits on the owner, across projects. The screen
// reads its project filter and the open item from the search params, so it
// mounts under Suspense like the other static-export pages.
export default function ForYouPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={4} />}>
      <ForYou />
    </Suspense>
  );
}
