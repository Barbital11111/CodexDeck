import { Modal, Radio, Typography } from "antd";
import type { AccountSummary, AccountsExportFormat } from "../types/app";
import type { MessageCatalog } from "../i18n/catalog";

type ExportFormatDialogProps = {
  open: boolean;
  account?: AccountSummary;
  accountKeys?: string[];
  exportFormat: AccountsExportFormat;
  exportingAccounts: boolean;
  copy: MessageCatalog["exportDialog"];
  onChangeFormat: (format: AccountsExportFormat) => void;
  onConfirm: () => void;
  onClose: () => void;
};

export function ExportFormatDialog({
  open,
  account,
  accountKeys,
  exportFormat,
  exportingAccounts,
  copy,
  onChangeFormat,
  onConfirm,
  onClose,
}: ExportFormatDialogProps) {
  return (
    <Modal
      title={copy.title}
      open={open}
      onOk={onConfirm}
      onCancel={onClose}
      okText={copy.ok}
      cancelText={copy.cancel}
      confirmLoading={exportingAccounts}
      destroyOnHidden
    >
      <div className="exportFormatDialog">
        <Typography.Text type="secondary">
          {account
            ? copy.singleDescription
            : accountKeys?.length
              ? copy.selectedDescription(accountKeys.length)
              : copy.allDescription}
        </Typography.Text>
        <Radio.Group
          className="exportFormatOptions"
          value={exportFormat}
          onChange={(event) => onChangeFormat(event.target.value as AccountsExportFormat)}
          options={[
            {
              value: "codexDeck",
              label: (
                <span className="exportFormatOption">
                  <strong>{copy.codexDeckTitle}</strong>
                  <span>{copy.codexDeckDescription}</span>
                </span>
              ),
            },
            {
              value: "sub2api",
              label: (
                <span className="exportFormatOption">
                  <strong>{copy.sub2apiTitle}</strong>
                  <span>{copy.sub2apiDescription}</span>
                </span>
              ),
            },
          ]}
        />
      </div>
    </Modal>
  );
}
