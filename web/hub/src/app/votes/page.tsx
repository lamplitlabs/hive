"use client";

import { Suspense } from "react";
import { SkeletonPage } from "@/components/hive/panel";
import { VotesPageClient } from "@/components/votes/votes";

export default function VotesPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={3} />}>
      <VotesPageClient />
    </Suspense>
  );
}
