// SPDX-License-Identifier: Apache-2.0

/** The lockup as the header shows it: the mark and the word in the page's ink. */
export function Logo() {
  return (
    <span className="inline-flex items-center gap-2 text-[15px] font-semibold tracking-[-0.035em]">
      <img src="/brand/mark.svg" alt="" width={24} height={24} />
      sariema
    </span>
  );
}
