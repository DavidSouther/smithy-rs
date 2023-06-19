/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

package software.amazon.smithy.rust.codegen.client.smithy.generators.client

import org.junit.jupiter.api.Test
import software.amazon.smithy.model.shapes.StructureShape
import software.amazon.smithy.rust.codegen.client.smithy.ClientCodegenContext
import software.amazon.smithy.rust.codegen.client.testutil.TestCodegenSettings
import software.amazon.smithy.rust.codegen.client.testutil.clientIntegrationTest
import software.amazon.smithy.rust.codegen.client.testutil.testSymbolProvider
import software.amazon.smithy.rust.codegen.core.rustlang.CargoDependency
import software.amazon.smithy.rust.codegen.core.rustlang.implBlock
import software.amazon.smithy.rust.codegen.core.rustlang.rust
import software.amazon.smithy.rust.codegen.core.rustlang.rustTemplate
import software.amazon.smithy.rust.codegen.core.rustlang.writable
import software.amazon.smithy.rust.codegen.core.smithy.RuntimeType
import software.amazon.smithy.rust.codegen.core.smithy.RustCrate
import software.amazon.smithy.rust.codegen.core.smithy.generators.BuilderGenerator
import software.amazon.smithy.rust.codegen.core.smithy.generators.StructureGenerator
import software.amazon.smithy.rust.codegen.core.testutil.TestWorkspace
import software.amazon.smithy.rust.codegen.core.testutil.asSmithyModel
import software.amazon.smithy.rust.codegen.core.testutil.compileAndTest
import software.amazon.smithy.rust.codegen.core.testutil.integrationTest
import software.amazon.smithy.rust.codegen.core.testutil.unitTest
import software.amazon.smithy.rust.codegen.core.util.lookup

class FluentClientGeneratorTest {
    val model = """
        namespace com.example
        use aws.protocols#awsJson1_0

        @awsJson1_0
        service HelloService {
            operations: [SayHello],
            version: "1"
        }

        operation SayHello { input: TestInput }
        structure TestInput {}
        
        @documentation("this documents the shape")
        structure MyStruct {
           foo: String,
           byteValue: Byte,
        }
    """.asSmithyModel()

    @Test
    fun `send() future implements Send`() {
        val test: (ClientCodegenContext, RustCrate) -> Unit = { codegenContext, rustCrate ->
            rustCrate.integrationTest("send_future_is_send") {
                val moduleName = codegenContext.moduleUseName()
                rustTemplate(
                    """
                    fn check_send<T: Send>(_: T) {}

                    ##[test]
                    fn test() {
                        let connector = #{TestConnection}::<#{SdkBody}>::new(Vec::new());
                        let config = $moduleName::Config::builder()
                            .endpoint_resolver("http://localhost:1234")
                            #{set_http_connector}
                            .build();
                        let smithy_client = aws_smithy_client::Builder::new()
                            .connector(connector.clone())
                            .middleware_fn(|r| r)
                            .build_dyn();
                        let client = $moduleName::Client::with_config(smithy_client, config);
                        check_send(client.say_hello().send());
                    }
                    """,
                    "TestConnection" to CargoDependency.smithyClient(codegenContext.runtimeConfig)
                        .withFeature("test-util").toType()
                        .resolve("test_connection::TestConnection"),
                    "SdkBody" to RuntimeType.sdkBody(codegenContext.runtimeConfig),
                    "set_http_connector" to writable {
                        if (codegenContext.smithyRuntimeMode.generateOrchestrator) {
                            rust(".http_connector(connector.clone())")
                        }
                    },
                )
            }
        }
        clientIntegrationTest(model, TestCodegenSettings.middlewareModeTestParams, test = test)
        clientIntegrationTest(
            model,
            TestCodegenSettings.orchestratorModeTestParams,
            test = test,
        )
    }

    private val struct = model.lookup<StructureShape>("com.example#MyStruct")

    @Test
    fun `generate inner builders`() {
        val provider = testSymbolProvider(model)
        val project = TestWorkspace.testProject(provider)
        project.moduleFor(struct) {
            rust("##![allow(deprecated)]")
            StructureGenerator(model, provider, this, struct, emptyList()).render()
            implBlock(provider.toSymbol(struct)) {
                BuilderGenerator.renderConvenienceMethod(this, provider, struct)
            }
            unitTest("generate_builders") {
                rust(
                    """
                    let my_struct_builder = MyStructBuilder::builder().byte_value(4).foo("hello!");
                    assert_eq!(*my_struct_builder.get_foo(), Some("hello!".to_string()));
                    let inner = my_struct_builder.inner();
                    assert_eq!(*inner.get_byte_value(), Some(4));
                    """,
                )
            }
        }
        project.withModule(provider.moduleForBuilder(struct)) {
            BuilderGenerator(model, provider, struct, emptyList()).render(this)
        }
        project.compileAndTest()
    }
}
