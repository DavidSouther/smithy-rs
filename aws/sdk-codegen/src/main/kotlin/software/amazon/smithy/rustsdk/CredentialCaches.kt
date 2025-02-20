/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

package software.amazon.smithy.rustsdk

import software.amazon.smithy.model.shapes.OperationShape
import software.amazon.smithy.rust.codegen.client.smithy.ClientCodegenContext
import software.amazon.smithy.rust.codegen.client.smithy.customize.ClientCodegenDecorator
import software.amazon.smithy.rust.codegen.client.smithy.generators.OperationCustomization
import software.amazon.smithy.rust.codegen.client.smithy.generators.OperationSection
import software.amazon.smithy.rust.codegen.client.smithy.generators.config.ConfigCustomization
import software.amazon.smithy.rust.codegen.client.smithy.generators.config.ServiceConfig
import software.amazon.smithy.rust.codegen.core.rustlang.Writable
import software.amazon.smithy.rust.codegen.core.rustlang.rust
import software.amazon.smithy.rust.codegen.core.rustlang.rustTemplate
import software.amazon.smithy.rust.codegen.core.rustlang.writable
import software.amazon.smithy.rust.codegen.core.smithy.RuntimeConfig
import software.amazon.smithy.rust.codegen.core.smithy.customize.AdHocCustomization
import software.amazon.smithy.rust.codegen.core.smithy.customize.adhocCustomization

class CredentialsCacheDecorator : ClientCodegenDecorator {
    override val name: String = "CredentialsCache"
    override val order: Byte = 0
    override fun configCustomizations(
        codegenContext: ClientCodegenContext,
        baseCustomizations: List<ConfigCustomization>,
    ): List<ConfigCustomization> {
        return baseCustomizations + CredentialCacheConfig(codegenContext)
    }

    override fun operationCustomizations(
        codegenContext: ClientCodegenContext,
        operation: OperationShape,
        baseCustomizations: List<OperationCustomization>,
    ): List<OperationCustomization> {
        return baseCustomizations + CredentialsCacheFeature(codegenContext.runtimeConfig)
    }

    override fun extraSections(codegenContext: ClientCodegenContext): List<AdHocCustomization> =
        listOf(
            adhocCustomization<SdkConfigSection.CopySdkConfigToClientConfig> { section ->
                rust("${section.serviceConfigBuilder}.set_credentials_cache(${section.sdkConfig}.credentials_cache().cloned());")
            },
        )
}

/**
 * Add a `.credentials_cache` field and builder to the `Config` for a given service
 */
class CredentialCacheConfig(codegenContext: ClientCodegenContext) : ConfigCustomization() {
    private val runtimeConfig = codegenContext.runtimeConfig
    private val runtimeMode = codegenContext.smithyRuntimeMode
    private val codegenScope = arrayOf(
        "CredentialsCache" to AwsRuntimeType.awsCredentialTypes(runtimeConfig).resolve("cache::CredentialsCache"),
        "DefaultProvider" to defaultProvider(),
        "SharedCredentialsCache" to AwsRuntimeType.awsCredentialTypes(runtimeConfig).resolve("cache::SharedCredentialsCache"),
        "SharedCredentialsProvider" to AwsRuntimeType.awsCredentialTypes(runtimeConfig).resolve("provider::SharedCredentialsProvider"),
    )

    override fun section(section: ServiceConfig) = writable {
        when (section) {
            ServiceConfig.ConfigStruct -> {
                if (runtimeMode.defaultToMiddleware) {
                    rustTemplate(
                        """pub(crate) credentials_cache: #{SharedCredentialsCache},""",
                        *codegenScope,
                    )
                }
            }

            ServiceConfig.ConfigImpl -> {
                if (runtimeMode.defaultToOrchestrator) {
                    rustTemplate(
                        """
                        /// Returns the credentials cache.
                        pub fn credentials_cache(&self) -> #{SharedCredentialsCache} {
                            self.inner.load::<#{SharedCredentialsCache}>().expect("credentials cache should be set").clone()
                        }
                        """,
                        *codegenScope,
                    )
                } else {
                    rustTemplate(
                        """
                        /// Returns the credentials cache.
                        pub fn credentials_cache(&self) -> #{SharedCredentialsCache} {
                            self.credentials_cache.clone()
                        }
                        """,
                        *codegenScope,
                    )
                }
            }

            ServiceConfig.BuilderStruct ->
                rustTemplate("credentials_cache: Option<#{CredentialsCache}>,", *codegenScope)

            ServiceConfig.BuilderImpl -> {
                rustTemplate(
                    """
                    /// Sets the credentials cache for this service
                    pub fn credentials_cache(mut self, credentials_cache: #{CredentialsCache}) -> Self {
                        self.set_credentials_cache(Some(credentials_cache));
                        self
                    }

                    /// Sets the credentials cache for this service
                    pub fn set_credentials_cache(&mut self, credentials_cache: Option<#{CredentialsCache}>) -> &mut Self {
                        self.credentials_cache = credentials_cache;
                        self
                    }
                    """,
                    *codegenScope,
                )
            }

            ServiceConfig.BuilderBuild -> {
                if (runtimeMode.defaultToOrchestrator) {
                    rustTemplate(
                        """
                        layer.store_put(
                            self.credentials_cache
                                .unwrap_or_else({
                                    let sleep = self.sleep_impl.clone();
                                    || match sleep {
                                        Some(sleep) => {
                                            #{CredentialsCache}::lazy_builder()
                                                .sleep(sleep)
                                                .into_credentials_cache()
                                        }
                                        None => #{CredentialsCache}::lazy(),
                                    }
                                })
                                .create_cache(self.credentials_provider.unwrap_or_else(|| {
                                    #{SharedCredentialsProvider}::new(#{DefaultProvider})
                                })),
                        );
                        """,
                        *codegenScope,
                    )
                } else {
                    rustTemplate(
                        """
                        credentials_cache: self
                            .credentials_cache
                            .unwrap_or_else({
                                let sleep = self.sleep_impl.clone();
                                || match sleep {
                                    Some(sleep) => {
                                        #{CredentialsCache}::lazy_builder()
                                            .sleep(sleep)
                                            .into_credentials_cache()
                                    }
                                    None => #{CredentialsCache}::lazy(),
                                }
                            })
                            .create_cache(
                                self.credentials_provider.unwrap_or_else(|| {
                                    #{SharedCredentialsProvider}::new(#{DefaultProvider})
                                })
                            ),
                        """,
                        *codegenScope,
                    )
                }
            }

            else -> emptySection
        }
    }
}

class CredentialsCacheFeature(private val runtimeConfig: RuntimeConfig) : OperationCustomization() {
    override fun section(section: OperationSection): Writable {
        return when (section) {
            is OperationSection.MutateRequest -> writable {
                rust(
                    """
                    #T(&mut ${section.request}.properties_mut(), ${section.config}.credentials_cache.clone());
                    """,
                    setCredentialsCache(runtimeConfig),
                )
            }

            else -> emptySection
        }
    }
}

fun setCredentialsCache(runtimeConfig: RuntimeConfig) =
    AwsRuntimeType.awsHttp(runtimeConfig).resolve("auth::set_credentials_cache")
