{{/*
Expand the name of the chart.
*/}}
{{- define "flume-rs.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "flume-rs.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "flume-rs.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels.
*/}}
{{- define "flume-rs.labels" -}}
helm.sh/chart: {{ include "flume-rs.chart" . }}
{{ include "flume-rs.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels (no component).
*/}}
{{- define "flume-rs.selectorLabels" -}}
app.kubernetes.io/name: {{ include "flume-rs.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
JobManager labels.
*/}}
{{- define "flume-rs.jobmanager.labels" -}}
{{ include "flume-rs.labels" . }}
app.kubernetes.io/component: jobmanager
{{- end }}

{{/*
JobManager selector labels.
*/}}
{{- define "flume-rs.jobmanager.selectorLabels" -}}
{{ include "flume-rs.selectorLabels" . }}
app.kubernetes.io/component: jobmanager
{{- end }}

{{/*
TaskManager labels.
*/}}
{{- define "flume-rs.taskmanager.labels" -}}
{{ include "flume-rs.labels" . }}
app.kubernetes.io/component: taskmanager
{{- end }}

{{/*
TaskManager selector labels.
*/}}
{{- define "flume-rs.taskmanager.selectorLabels" -}}
{{ include "flume-rs.selectorLabels" . }}
app.kubernetes.io/component: taskmanager
{{- end }}

{{/*
Service account name.
*/}}
{{- define "flume-rs.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "flume-rs.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
JobManager image.
*/}}
{{- define "flume-rs.jobmanager.image" -}}
{{- $repo := .Values.jobmanager.image.repository | default (printf "%s/flume-server" .Values.image.registry) -}}
{{- $tag := .Values.image.tag | default .Chart.AppVersion -}}
{{- printf "%s:%s" $repo $tag -}}
{{- end }}

{{/*
TaskManager image.
*/}}
{{- define "flume-rs.taskmanager.image" -}}
{{- $repo := .Values.taskmanager.image.repository | default (printf "%s/flume-tm" .Values.image.registry) -}}
{{- $tag := .Values.image.tag | default .Chart.AppVersion -}}
{{- printf "%s:%s" $repo $tag -}}
{{- end }}
