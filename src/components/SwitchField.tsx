import { Switch } from "antd";

type SwitchFieldProps = {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  checkedText: string;
  uncheckedText: string;
  disabled?: boolean;
  loading?: boolean;
  rowClassName?: string;
};

export function SwitchField({
  checked,
  onChange,
  label,
  checkedText,
  uncheckedText,
  disabled = false,
  loading = false,
  rowClassName,
}: SwitchFieldProps) {
  return (
    <div className={["settingRow", rowClassName].filter(Boolean).join(" ")}>
      <div className="settingMeta">
        <strong>{label}</strong>
      </div>
      <label className="themeSwitch antdSwitchWrap" aria-label={label}>
        <Switch
          checked={checked}
          disabled={disabled}
          loading={loading}
          onChange={onChange}
        />
        <span className="themeSwitchText">{checked ? checkedText : uncheckedText}</span>
      </label>
    </div>
  );
}
