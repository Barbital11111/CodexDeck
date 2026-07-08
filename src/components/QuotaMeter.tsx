type QuotaMeterProps = {
  label: string;
  variant: "bar" | "stat";
  percent?: number | null;
  totalPercent?: number | null;
  usedText?: string | null;
  totalText?: string | null;
  value?: string | null;
  caption?: string | null;
  tone?: "auto" | "good" | "warn" | "critical" | "neutral";
  size?: "md" | "sm";
};

function normalizePercent(value: number | null | undefined) {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return null;
  }
  return Math.max(0, value);
}

function toneFromPercent(value: number | null, explicitTone: QuotaMeterProps["tone"]) {
  if (explicitTone && explicitTone !== "auto") {
    return explicitTone;
  }
  if (value === null) {
    return "neutral";
  }
  if (value >= 60) {
    return "good";
  }
  if (value >= 30) {
    return "warn";
  }
  return "critical";
}

export function QuotaMeter({
  label,
  variant,
  percent,
  totalPercent,
  usedText,
  totalText,
  value,
  caption,
  tone = "auto",
  size = "md",
}: QuotaMeterProps) {
  const normalizedPercent = normalizePercent(percent);
  const normalizedTotal = Math.max(1, normalizePercent(totalPercent) ?? 100);
  const visualPercent =
    normalizedPercent === null
      ? 0
      : Math.max(0, Math.min(100, (normalizedPercent / normalizedTotal) * 100));
  const resolvedTone = toneFromPercent(normalizedPercent, tone);
  const valueText =
    usedText && totalText ? `${usedText} / ${totalText}` : usedText || totalText || null;

  if (variant === "stat") {
    return (
      <div className={`quotaMeter quotaMeter-${resolvedTone} quotaMeter-${size} quotaMeter-stat`}>
        <span>{label}</span>
        <strong>{value || "--"}</strong>
        {caption ? <small>{caption}</small> : null}
      </div>
    );
  }

  return (
    <div className={`quotaMeter quotaMeter-${resolvedTone} quotaMeter-${size}`}>
      <div className="quotaMeterLabelRow">
        <span>{label}</span>
        {caption ? <small>{caption}</small> : null}
      </div>
      <div className="quotaMeterValueRow">
        <strong>{normalizedPercent === null ? "--" : `${normalizedPercent.toFixed(0)}%`}</strong>
        {valueText ? <em>{valueText}</em> : null}
      </div>
      <div
        className="quotaMeterTrack"
        role="meter"
        aria-valuemin={0}
        aria-valuemax={normalizedTotal}
        aria-valuenow={normalizedPercent ?? 0}
      >
        <span style={{ width: `${visualPercent}%` }} />
      </div>
    </div>
  );
}
