import Foundation

enum NativeMenuProgressTone: String, Codable, Hashable {
    case high
    case medium
    case low
    case critical
}

struct NativeMenuStrings: Codable {
    let view_recommended: String
    let back_to_current: String
    let switch_to_viewed: String
    let refresh: String
    let open_qiehuan_yingyong: String
    let open_details: String
    let view_all_accounts: String
    let settings: String
    let quit: String
    let empty_title: String
    let empty_desc: String

    private enum CodingKeys: String, CodingKey {
        case view_recommended
        case back_to_current
        case switch_to_viewed
        case refresh
        case open_qiehuan_yingyong
        case legacyOpenCockpitTools = "open_cockpit_tools"
        case open_details
        case view_all_accounts
        case settings
        case quit
        case empty_title
        case empty_desc
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.view_recommended = try container.decode(String.self, forKey: .view_recommended)
        self.back_to_current = try container.decode(String.self, forKey: .back_to_current)
        self.switch_to_viewed = try container.decode(String.self, forKey: .switch_to_viewed)
        self.refresh = try container.decode(String.self, forKey: .refresh)
        if let currentLabel = try container.decodeIfPresent(
            String.self,
            forKey: .open_qiehuan_yingyong
        ) {
            self.open_qiehuan_yingyong = currentLabel
        } else {
            self.open_qiehuan_yingyong = try container.decode(String.self, forKey: .legacyOpenCockpitTools)
        }
        self.open_details = try container.decode(String.self, forKey: .open_details)
        self.view_all_accounts = try container.decode(String.self, forKey: .view_all_accounts)
        self.settings = try container.decode(String.self, forKey: .settings)
        self.quit = try container.decode(String.self, forKey: .quit)
        self.empty_title = try container.decode(String.self, forKey: .empty_title)
        self.empty_desc = try container.decode(String.self, forKey: .empty_desc)
    }
}

struct NativeMenuQuotaRow: Codable, Hashable {
    let label: String
    let value: String
    let progress: Int?
    let progress_tone: NativeMenuProgressTone?
    let subtext: String?
}

struct NativeMenuAccountCard: Codable, Hashable, Identifiable {
    let id: String
    let title: String
    let plan: String?
    let updated_text: String
    let quota_rows: [NativeMenuQuotaRow]
}

struct NativeMenuPlatform: Codable, Hashable, Identifiable {
    let id: String
    let title: String
    let short_title: String
    let nav_target: String
    let accent_hex: String
    let current_account_id: String?
    let recommended_account_id: String?
    let cards: [NativeMenuAccountCard]

    var currentOrFirstAccountId: String? {
        if let current_account_id, self.cards.contains(where: { $0.id == current_account_id }) {
            return current_account_id
        }
        return self.cards.first?.id
    }
}

struct NativeMenuSnapshot: Codable {
    let strings: NativeMenuStrings
    let platforms: [NativeMenuPlatform]
    let selected_platform_id: String
    /// 开启菜单栏额度时：打开托盘菜单应强制选中配置平台并展示当前账号。
    let prefer_selected_platform: Bool?

    var shouldPreferSelectedPlatform: Bool {
        self.prefer_selected_platform ?? false
    }
}
