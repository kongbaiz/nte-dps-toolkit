// Developer-only reference using the production primitives, not a parallel UI kit.
import { useState } from "react";
import { FormRow } from "@/components/nte/form-row";
import {
  PageDescription,
  PageHeader,
  PageScrollArea,
  PageTitle,
  PreferencesGrid,
} from "@/components/nte/page-layout";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input, NativeSelect } from "@/components/ui/form-controls";
import { Switch } from "@/components/ui/switch";

export function UiFoundationPreview() {
  const [checked, setChecked] = useState(true);
  const params = new URLSearchParams(location.search);
  const density = params.get("density") ?? "cozy";
  return (
    <section className="console-workspace flex h-full min-h-0 flex-col text-foreground">
      <PageHeader className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <PageTitle>UI 规范预览</PageTitle>
          <PageDescription>
            真实组件 · 主题、密度和局部覆盖在这里一起检查
          </PageDescription>
        </div>
        <NativeSelect
          aria-label="预览密度"
          className="w-36"
          value={density}
          onChange={(event) => {
            params.set("density", event.target.value);
            location.search = params.toString();
          }}
        >
          <option value="compact">紧凑</option>
          <option value="cozy">适中</option>
          <option value="comfortable">宽松</option>
        </NativeSelect>
      </PageHeader>
      <PageScrollArea>
        <PreferencesGrid>
          <div className="ui-section-stack">
            <Card>
              <CardHeader>
                <CardTitle>表单与状态</CardTitle>
                <CardDescription>
                  相同高度、边框、焦点和禁用状态；窄容器自动堆叠。
                </CardDescription>
              </CardHeader>
              <CardContent className="divide-y">
                <FormRow
                  label="显示名称"
                  description="说明文字与正文使用固定字号，不随密度缩放。"
                >
                  <Input aria-label="显示名称" defaultValue="NTE DPS TOOL" />
                </FormRow>
                <FormRow label="选择项">
                  <NativeSelect aria-label="选择项" defaultValue="default">
                    <option value="default">默认值</option>
                    <option value="other">另一项</option>
                  </NativeSelect>
                </FormRow>
                <FormRow label="禁用">
                  <Input aria-label="禁用字段" disabled value="不可编辑" />
                </FormRow>
                <FormRow label="无效输入">
                  <Input
                    aria-label="无效输入"
                    aria-invalid="true"
                    defaultValue="示例错误"
                  />
                </FormRow>
                <FormRow label="开关">
                  <Switch
                    aria-label="示例开关"
                    checked={checked}
                    onCheckedChange={setChecked}
                  />
                </FormRow>
              </CardContent>
              <CardFooter className="flex-wrap justify-end gap-2">
                <Button variant="outline">次要操作</Button>
                <Button>主要操作</Button>
              </CardFooter>
            </Card>
            <Card>
              <CardHeader>
                <CardTitle>按钮层级</CardTitle>
              </CardHeader>
              <CardContent className="flex flex-wrap items-center gap-2">
                <Button>默认</Button>
                <Button variant="outline">描边</Button>
                <Button variant="secondary">次要</Button>
                <Button variant="ghost">轻量</Button>
                <Button variant="destructive">危险</Button>
                <Button disabled>禁用</Button>
              </CardContent>
            </Card>
          </div>
          <div className="ui-section-stack">
            <Card size="sm">
              <CardHeader>
                <CardTitle>紧凑卡片</CardTitle>
                <CardDescription>
                  用 size="sm"，不通过父页面强制覆盖 card-spacing。
                </CardDescription>
              </CardHeader>
              <CardContent className="text-sm">
                共享颜色沿用 shadcn 语义 token；业务页面只负责内容与排列。
              </CardContent>
            </Card>
            <Card
              className="[--card-spacing:8px]"
              data-testid="custom-spacing-card"
            >
              <CardHeader>
                <CardTitle>显式局部覆盖</CardTitle>
                <CardDescription>
                  此卡片显式设置 8px 内边距；下面的输入框显式设置 40px
                  高度。组件层不会把它们改回去。
                </CardDescription>
              </CardHeader>
              <CardContent>
                <Input
                  className="h-10"
                  aria-label="局部高度覆盖"
                  defaultValue="className 覆盖有效"
                />
              </CardContent>
            </Card>
            <Card>
              <CardHeader>
                <CardTitle>修改入口</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-sm">
                <p>颜色、密度、宽度和尺寸：styles/theme.css</p>
                <p>通用布局与表单：styles/ui-foundation.css</p>
                <p>控制台外壳与业务布局：styles/console.css</p>
                <p>组件行为与变体：components/ui 与 components/nte</p>
              </CardContent>
            </Card>
          </div>
        </PreferencesGrid>
      </PageScrollArea>
    </section>
  );
}
